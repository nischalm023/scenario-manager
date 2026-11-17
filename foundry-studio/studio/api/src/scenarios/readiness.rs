//! Within `scenarios`, `readiness` is spec 07 section 3.8's `Readiness` of one version at its site's heads of
//! `main`: one `resolve_cut` per read, then the export and `check` on the compute pool, cached by the cut, the
//! version and its revision, the crate's version and the effective runner image, so it reruns only when one of them
//! changes. A site that does not resolve, or a world-proto-server that does not answer, is reported as the refusal it
//! is, never as "not ready yet".

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::Uri;
use axum::{Extension, Json};
use rustc_hash::FxHashMap;
use scenario_spec::{GENERATOR_VERSION, this_build_accepts};
use sqlx::PgPool;
use studio_protocol::{
    BranchName, Pin, ProblemItem, Readiness, Revision, RunOverrides, ScenarioRow, ScopePath, Seq, Severity, SitePath,
    Ulid, UnitCut, VersionNumber, WallClock,
};
use tokio::sync::OnceCell;
use wps_client::compute::run;
use wps_client::wps::Wps;
use wps_client::{Client, Compute, CutExport, Exactness, UnitCut as WpsCut, WorldStore, WpsError};

use crate::GateState;
use crate::problem::{ProblemAnswer, internal, validation_failed, wps_problem};

use super::check::{CheckOutcome, check};
use super::reads::{VersionQuery, no_version};
use super::rows::{self, VersionRecord};
use super::{compute, id_in, not_found};

/// What readiness reads of a site: one consistent cut of its scopes at `main`, and the export of that cut.
pub trait SiteReader: Send + Sync {
    fn resolve_cut(&self, site: &SitePath) -> impl Future<Output = Result<WpsCut, WpsError>> + Send;
    fn export(&self, cut: &WpsCut) -> impl Future<Output = Result<CutExport, WpsError>> + Send;
}

impl<S: Wps> SiteReader for Client<S> {
    async fn resolve_cut(&self, site: &SitePath) -> Result<WpsCut, WpsError> {
        WorldStore::resolve_cut(self, site.as_scope(), &Pin::Branch(BranchName::main())).await
    }

    async fn export(&self, cut: &WpsCut) -> Result<CutExport, WpsError> {
        WorldStore::export(self, cut).await
    }
}

/// What a readiness check is keyed by: the cut's changesets, the version at its revision, the generator crate's
/// version and the runner image's name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CheckKey {
    // ordered: the cut's own map, which hashes its scopes in path order, as a key must.
    seqs: BTreeMap<ScopePath, Seq>,
    scenario: Ulid,
    version: VersionNumber,
    revision: Revision,
    crate_version: &'static str,
    image: String,
}

/// The checks this router has run, one per key, each run once however many reads wait on it, against the runner
/// image a plan would name (spec 08 section 3.5: the first cut's one local image, by its digest).
#[derive(Default)]
pub struct ReadinessCache {
    entries: Mutex<FxHashMap<CheckKey, Arc<OnceCell<Arc<CheckOutcome>>>>>,
    computed: AtomicUsize,
    image: String,
}

/// More keys than this and the cache starts again: every key holds one version at one cut, which a person moves on.
const KEPT: usize = 512;

impl ReadinessCache {
    /// A cache whose checks are keyed by `image`, the image a plan would name now.
    pub fn of_image(image: &str) -> Self {
        Self { image: image.to_owned(), ..Self::default() }
    }

    /// How many checks have run: the generator and the reader run once per check.
    pub fn computations(&self) -> usize {
        self.computed.load(Ordering::SeqCst)
    }

    fn cell(&self, key: CheckKey) -> Arc<OnceCell<Arc<CheckOutcome>>> {
        let mut entries = self.entries.lock().expect("the readiness cache's lock");
        if entries.len() >= KEPT {
            entries.clear();
        }
        entries.entry(key).or_default().clone()
    }
}

/// Version `version` of `scenario` at its site's heads of `main`: errors only when the compute pool is full, and
/// every other failure is a problem of the readiness answered.
async fn readiness<R: SiteReader>(
    reader: &R,
    pool: Arc<dyn Compute>,
    cache: &ReadinessCache,
    scenario: &ScenarioRow,
    version: &VersionRecord,
) -> Result<Readiness, WpsError> {
    let checked_at = WallClock::now();
    let cut = match reader.resolve_cut(&scenario.site_scope).await {
        Ok(cut) => cut,
        Err(fault) => return Ok(refusal(checked_at, None, &fault)),
    };
    let key = CheckKey {
        seqs: cut.seqs.clone(),
        scenario: scenario.id.clone(),
        version: version.n,
        revision: version.revision,
        crate_version: GENERATOR_VERSION,
        image: cache.image.clone(),
    };
    let content = version.content.clone();
    let cell = cache.cell(key);
    let outcome = cell
        .get_or_try_init(|| async {
            let export = reader.export(&cut).await?;
            let accepts = this_build_accepts();
            let outcome =
                run(&*pool, move || check(&export.bytes, &content, &RunOverrides::default(), &accepts)).await?;
            cache.computed.fetch_add(1, Ordering::SeqCst);
            Ok::<_, WpsError>(Arc::new(outcome))
        })
        .await;
    match outcome {
        Ok(outcome) => {
            let ready = !outcome.problems.iter().any(|problem| problem.severity == Severity::Error);
            let (problems, workload) = (outcome.problems.clone(), outcome.workload.clone());
            Ok(Readiness { checked_at, cut: Some(answered(&cut)), ready, workload, problems })
        }
        Err(busy @ WpsError::ComputeBusy { .. }) => Err(busy),
        Err(fault) => Ok(refusal(checked_at, Some(answered(&cut)), &fault)),
    }
}

/// `GET /api/scenarios/{s}/readiness?version=`: the readiness of a version, the latest by default.
pub async fn route<S: Wps>(
    State(state): State<GateState<S>>,
    Extension(cache): Extension<Arc<ReadinessCache>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<VersionQuery>, QueryRejection>,
) -> Result<Json<Readiness>, ProblemAnswer> {
    let instance = uri.path();
    let id = id_in(path, "scenario", instance)?;
    let Query(VersionQuery { version }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let version = version.map(VersionNumber);
    read(&state.app.pool, &*state.world, compute(&state), &cache, &id, version, instance).await.map(Json)
}

/// The readiness of version `version` of scenario `id`, the latest by default, read through `reader`.
pub async fn read<R: SiteReader>(
    pg: &PgPool,
    reader: &R,
    pool: Arc<dyn Compute>,
    cache: &ReadinessCache,
    id: &Ulid,
    version: Option<VersionNumber>,
    instance: &str,
) -> Result<Readiness, ProblemAnswer> {
    let fault = |fault| internal("read the scenario", fault, instance);
    let mut conn = pg.acquire().await.map_err(fault)?;
    let row =
        rows::scenario(&mut conn, id, false).await.map_err(fault)?.ok_or_else(|| not_found("scenario", instance))?;
    let n = version.unwrap_or(row.latest_version);
    let record = rows::version(&mut conn, id, n, false).await.map_err(fault)?.ok_or_else(|| no_version(n, instance))?;
    drop(conn);
    readiness(reader, pool, cache, &row, &record).await.map_err(|fault| wps_problem(&fault, instance))
}

/// A readiness that could not read the site, the cut, or its export: the failure as its one problem.
fn refusal(checked_at: WallClock, cut: Option<UnitCut>, fault: &WpsError) -> Readiness {
    let problem = wps_problem(fault, "").0;
    let scope = match fault {
        WpsError::PinUnresolved { scope, .. } => Some(scope.clone()),
        _ => None,
    };
    let code = serde_json::to_value(problem.type_).ok().and_then(|code| code.as_str().map(str::to_owned));
    let item = ProblemItem {
        code: code.unwrap_or_else(|| "wps-fault".to_owned()),
        severity: Severity::Error,
        ref_: None,
        scope,
        field: None,
        knob: None,
        text: problem.detail,
    };
    Readiness { checked_at, cut, ready: false, workload: None, problems: vec![item] }
}

/// Spec 04's cut as the API answers it.
pub fn answered(cut: &WpsCut) -> UnitCut {
    let (exact, exact_reason) = match &cut.exact {
        Exactness::Exact => (true, None),
        Exactness::ComponentsOnly { reason } => (false, Some(reason.clone())),
    };
    UnitCut {
        unit: cut.unit.clone(),
        requested: cut.requested.clone(),
        branch: cut.branch.clone(),
        seqs: cut.seqs.clone(),
        exact,
        exact_reason,
    }
}
