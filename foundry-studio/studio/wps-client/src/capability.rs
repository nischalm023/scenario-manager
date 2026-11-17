//! Within `wps-client`, `capability` is spec 04 section 3.12's detection: a probe at start that writes nothing, read
//! against `GET /protocol`'s `features` where WPS lists them and against the revision dev.sh names in
//! `STUDIO_WPS_REVISION`, looked up in the table of revisions this crate was checked against. A listed revision and
//! the server's features must agree, or the client stops naming both; a server without scopes phase 3 (world-proto
//! 092d695) is refused at start, since every write needs it (the lead's decision of 2026-10-19), and so is one
//! without structured refusals (b9d82a8), since the client reads every refusal by its code (`refusal`).

use rustc_hash::FxHashSet;

use crate::error::WpsError;
use crate::value::{Capabilities, Capability};
use crate::wps::Wps;

/// The variable naming the world-proto revision the configured server was built from.
pub const REVISION_VAR: &str = "STUDIO_WPS_REVISION";

/// The protocol minor foundry's reader accepts (`sim/site-model/src/document.rs`).
pub const READER_MINOR: &str = "0.7";

/// The revisions this crate was checked against, each with what it has landed: world-proto master cd4923516,
/// world-proto#4's merge, has the export's flags beside structured refusals on scopes phase 3 (`scope_changesets`),
/// the first cut and the conveniences, listing them as `features`. A server without structured refusals is refused
/// at start, and one without the export's flags when a site is exported.
pub const KNOWN_REVISIONS: &[(&str, &[Capability])] = &[(
    "cd4923516",
    &[
        Capability::Generation,
        Capability::PinKeepingGeneration,
        Capability::GeneratorModules,
        Capability::ScopePaths,
        Capability::KinReferences,
        Capability::RegistryRevision,
        Capability::ListingNames,
        Capability::ExactReplace,
        Capability::OperationId,
        Capability::MultiScopeChangesets,
        Capability::StructuredRefusals,
        Capability::ExportFlags,
    ],
)];

/// What `revision` (a short or full commit) is known to have landed, if this crate was checked against it.
pub fn known(revision: &str) -> Option<&'static [Capability]> {
    let revision = revision.trim();
    if revision.len() < 7 {
        return None;
    }
    KNOWN_REVISIONS
        .iter()
        .find(|(known, _)| known.starts_with(revision) || revision.starts_with(known))
        .map(|(_, landed)| *landed)
}

/// Probe the server, writing nothing, and settle what it has landed; stop on anything this client cannot work with.
pub async fn probe<S: Wps>(wps: &S, revision: Option<&str>) -> Result<Capabilities, WpsError> {
    let incompatible = |text: String| WpsError::Incompatible { text };
    let protocol = wps.protocol().await?;
    let minor: String = protocol.proto_version.split('.').take(2).collect::<Vec<_>>().join(".");
    if minor != READER_MINOR {
        return Err(incompatible(format!(
            "it speaks protocol {}, and foundry's reader accepts {READER_MINOR} only, so no export of it would load in \
             the simulator",
            protocol.proto_version
        )));
    }
    let scopes = wps.scopes().await.map_err(|fault| {
        incompatible(format!(
            "its scope listing does not read as paths with flags and units, as world-proto-server answers since \
             c384e59, and this client is built on paths alone: {fault}"
        ))
    })?;
    let mut probed = FxHashSet::default();
    for row in &scopes {
        if row.boundary.is_none() && !row.scope.starts_with("registry/") {
            return Err(incompatible(format!(
                "its listing gives {} no boundary flag, as a server before c384e59",
                row.scope
            )));
        }
    }
    probed.insert(Capability::ScopePaths);
    if wps.generators().await.is_ok() {
        probed.extend([Capability::Generation, Capability::GeneratorModules, Capability::PinKeepingGeneration]);
    }
    // A server listing no `features` predates every revision the table knows, so a configured one vouches for none.
    let landed: FxHashSet<Capability> = match &protocol.features {
        Some(features) => {
            let listed: FxHashSet<Capability> =
                features.iter().filter_map(|word| Capability::from_feature(word)).collect();
            if let Some(missing) = [Capability::ScopePaths, Capability::Generation]
                .into_iter()
                .find(|capability| listed.contains(capability) != probed.contains(capability))
            {
                return Err(incompatible(format!(
                    "its `features` and the probe disagree about {missing:?}: features {features:?}"
                )));
            }
            if let Some(revision) = revision
                && let Some(table) = known(revision)
                && let Some(differs) = table.iter().chain(&listed).copied().find(|capability| {
                    capability.feature_word().is_some() && table.contains(capability) != listed.contains(capability)
                })
            {
                return Err(incompatible(format!(
                    "STUDIO_WPS_REVISION={revision} names a revision with {table:?}, and the server lists the features \
                     {features:?}; they disagree about {differs:?}"
                )));
            }
            listed.union(&probed).copied().collect()
        }
        None => probed,
    };
    for needed in [Capability::Generation, Capability::MultiScopeChangesets, Capability::StructuredRefusals] {
        if !landed.contains(&needed) {
            return Err(WpsError::Unsupported { capability: needed });
        }
    }
    let server_version = wps.healthz().await.unwrap_or_else(|_| "unknown".to_owned());
    Ok(Capabilities { landed, server_version, proto_version: protocol.proto_version })
}
