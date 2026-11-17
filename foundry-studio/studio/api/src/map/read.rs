//! Within `map`, a `read` is a site at a cut as the Map panel and its references read it (spec 03 sections 3.5 and
//! 3.6): the location's cut resolved, its flagged export, the export's floor key, and the export `loaded` on the
//! compute pool with the site's name, its floor going to the client's floor cache under that key. Loading is the
//! costly part (about half a second of wall clock for a small shell), so one load per key is kept and shared by every
//! route that asks while it runs.

use std::sync::Arc;

use studio_protocol::{BranchName, KinPins, Location, Pin, Problem, ProblemType, ScopePath, SitePath};
use wps_client::cache::{Flight, FloorKey};
use wps_client::compute::run as on_pool;
use wps_client::site::site_name;
use wps_client::wps::Wps;
use wps_client::{UnitCut, WorldStore, WpsError};

use super::loaded::{Loaded, load};
use crate::GateState;
use crate::floors::{build_commit, floor_key};
use crate::problem::{ProblemAnswer, wps_problem};

/// The reads kept between requests, 64 MiB of them by their estimate.
pub struct Kept {
    reads: Flight<FloorKey, Arc<Result<Loaded, String>>>,
}

impl Default for Kept {
    fn default() -> Self {
        Self { reads: Flight::new(64 << 20, estimate) }
    }
}

/// A site at a cut, as one request read it.
pub struct SiteAt {
    pub location: Location,
    pub kin: Option<KinPins>,
    pub site: SitePath,
    pub cut: UnitCut,
    pub key: FloorKey,
    /// The loaded export, or why foundry's reader refused it.
    pub loaded: Arc<Result<Loaded, String>>,
}

/// Read `location` (with `kin`) as its site's map at a cut, kept or loaded now.
pub async fn site_at<S: Wps>(
    state: &GateState<S>,
    location: &str,
    kin: Option<&str>,
    instance: &str,
) -> Result<SiteAt, ProblemAnswer> {
    let named: Location = location.parse().map_err(|fault| malformed(fault, instance))?;
    let (site, map) = map_of(&named, instance)?;
    // A branch carries over from a site's root to its map (rule 1); a changeset is the root's own, and stays named.
    let location = match named.pin() {
        Pin::Changeset(_) => named,
        pin => Location::new(map.clone(), pin.clone()),
    };
    let kin: Option<KinPins> = kin.map(str::parse).transpose().map_err(|fault| malformed(fault, instance))?;
    if let Some(pins) = &kin {
        pins.check_beside(&location, site.as_scope()).map_err(|fault| malformed(fault, instance))?;
    }
    let cut = cut_of(state, &site, &location, instance).await?;
    let export = state.world.export(&cut).await.map_err(|error| world_problem(&error, instance))?;
    let key = floor_key(&export.sha256, build_commit(), None);
    state.app.floor_origins.remember(key.clone(), cut.clone());
    // The cache owns the load, as it owns a floor's compile: a request that stops waiting leaves it running.
    let (pool, world, held, root) =
        (state.app.compute.clone(), state.world.clone(), key.clone(), site.as_scope().clone());
    let loaded = state
        .app
        .maps
        .reads
        .get_or_spawn(key.clone(), move || async move {
            // The export carries no prologue, so the name is read as the site list reads it.
            let tree = world.subtree(&root).await?;
            let listed =
                tree.get(&root).ok_or_else(|| WpsError::NotFound { text: format!("there is no scope {root}") })?;
            let name = site_name(&world, listed).await?;
            let (read, floor) = on_pool(&*pool, move || load(&export, &map, name)).await?;
            if let Some(floor) = floor {
                world.floors().keep(held, floor);
            }
            Ok::<_, WpsError>(Arc::new(read))
        })
        .await
        .map_err(|error| wps_problem(&error, instance))?;
    Ok(SiteAt { location, kin, site, cut, key, loaded })
}

/// The site whose map `location` shows, and that map scope: a site's root opens its map (spec 03 section 3.2).
fn map_of(location: &Location, instance: &str) -> Result<(SitePath, ScopePath), ProblemAnswer> {
    let slugs: Vec<&str> = location.scope().slugs().collect();
    let site = slugs.get(..4).map(|root| root.join("/")).and_then(|root| root.parse::<SitePath>().ok());
    let mismatch = |detail: String| ProblemAnswer::from(Problem::new(ProblemType::PanelMismatch, detail).at(instance));
    match (site, slugs.len()) {
        (Some(site), 4) => {
            let map = site.as_scope().join(&"map".parse().expect("a slug")).expect("a site's map scope");
            Ok((site, map))
        }
        (Some(site), 5) if slugs[4] == "map" => Ok((site, location.scope().clone())),
        (Some(_), _) => Err(mismatch(format!(
            "The Map panel shows a site's map scope, and {} is not one; its own panel shows it.",
            location.scope()
        ))),
        (None, _) => Err(mismatch(format!(
            "The Map panel shows a site's map, and {} is no site's scope; a template's map is drawn by its evaluation.",
            location.scope()
        ))),
    }
}

/// The cut a location names: every scope of the site at a branch's heads. Until world-proto-server keeps one commit
/// order across a site's scopes (phase 5), a site of several scopes is read only at a branch head (the lead's decisions,
/// entries 35 and 36): a changeset pin, with `kin` or without, would leave the scopes it does not name at their heads,
/// so it is refused naming that rule (spec 03 statement 11).
async fn cut_of<S: Wps>(
    state: &GateState<S>,
    site: &SitePath,
    location: &Location,
    instance: &str,
) -> Result<UnitCut, ProblemAnswer> {
    let mut scope = site.as_scope();
    if let Pin::Changeset(seq) = location.pin() {
        let tree = state.world.tree().await.map_err(|error| world_problem(&error, instance))?;
        if tree.unit(scope).len() > 1 {
            let detail = format!(
                "Until world-proto-server keeps one commit order across a site's scopes (phase 5), a site of several \
                 scopes is read only at a branch head, and {location} pins changeset {seq}; open {site}'s map at @{}.",
                BranchName::main()
            );
            return Err(Problem::new(ProblemType::WpsNotReady, detail).at(instance).into());
        }
        scope = location.scope();
    }
    state.world.resolve_cut(scope, location.pin()).await.map_err(|error| world_problem(&error, instance))
}

/// A world-proto-server failure, an unknown scope said as spec 03 section 3.8 says it.
fn world_problem(error: &WpsError, instance: &str) -> ProblemAnswer {
    match error {
        WpsError::NotFound { text } => {
            Problem::new(ProblemType::LocationUnresolved, format!("There is {text}.")).at(instance).into()
        }
        other => wps_problem(other, instance),
    }
}

fn malformed(fault: impl ToString, instance: &str) -> ProblemAnswer {
    Problem::new(ProblemType::RefMalformed, fault.to_string()).at(instance).into()
}

/// A read's size, as the cache's budget counts it: a few hundred bytes an entity.
fn estimate(read: &Arc<Result<Loaded, String>>) -> usize {
    read.as_ref().as_ref().map_or(256, |loaded| 256 + loaded.entities.len() * 320)
}
