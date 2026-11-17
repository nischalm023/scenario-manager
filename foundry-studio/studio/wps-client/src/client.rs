//! Within `wps-client`, `client` is spec 04 section 3.2's `WorldStore`, the contract every consumer codes against,
//! and `Client`, its one implementation over any `Wps`: started by a probe that writes nothing, it resolves pins once
//! into cuts from one consistent read of a subtree's heads (`GET /scopes?under=` since scopes phase 3), reads a
//! site's scopes at a cut, reads a cut's flagged export through the export cache, which owns each read and its check
//! on the compute pool, and hands writes to `apply` and `check` in their own modules.

use std::collections::BTreeMap;
use std::env;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use studio_protocol::{BranchName, DeclaredId, EntityId, Pin, ScopePath, Seq, TagName};
use url::Url;

use crate::apply::NotLanded;
use crate::cache::{DEFAULT_BUDGET, ExportCache, ExportKey, FloorCache, export_cache, floor_cache};
use crate::capability::{REVISION_VAR, probe};
use crate::compute::Compute;
use crate::draft::{Applied, ChangesetDraft, ImportProposal, ScopedOp};
use crate::error::{Problem, WpsError};
use crate::events::{EventHub, ScopeEvents};
use crate::export::flagged;
use crate::http::{HttpConfig, HttpWps};
use crate::kinship::ScopeTree;
use crate::routes::{ROUTES_VAR, RouteShape, Routes};
use crate::schema::{PRELOAD, SchemaCache};
use crate::snapshot::{CutExport, DeclaredEntity, EntityReading, Record, ScopeSnapshot, UnitSnapshot};
use crate::value::{
    Author, Capabilities, Capability, DerivedListing, Exactness, LogEntry, Page, Prologue, Referrer, ScopeListing,
    UnitCut,
};
use crate::wire::{EntityQuery, WireChangeset};
use crate::wps::Wps;

/// The variable naming WPS's address; dev.sh sets it to the server it starts (ruling D109).
pub const URL_VAR: &str = "STUDIO_WPS_URL";
const DEFAULT_URL: &str = "http://127.0.0.1:7413";

/// Who checked a draft: this client alone until WPS's preview lands (spec 04 section 3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckedBy {
    AgentStudio,
    WorldProtoServer,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Preview {
    pub checked_by: CheckedBy,
    pub problems: Vec<Problem>,
}

/// The world graph as every consumer reads and writes it (spec 04 section 3.2).
pub trait WorldStore: Send + Sync {
    fn capabilities(&self) -> impl Future<Output = Result<Capabilities, WpsError>> + Send;
    fn list_scopes(&self, under: &ScopePath) -> impl Future<Output = Result<Vec<ScopeListing>, WpsError>> + Send;
    fn ensure_scope(
        &self,
        path: &ScopePath,
        boundary: bool,
    ) -> impl Future<Output = Result<ScopeListing, WpsError>> + Send;
    fn resolve_pin(&self, scope: &ScopePath, pin: &Pin) -> impl Future<Output = Result<Seq, WpsError>> + Send;
    fn resolve_cut(&self, anchor: &ScopePath, pin: &Pin) -> impl Future<Output = Result<UnitCut, WpsError>> + Send;
    fn resolve_fixed_cut(
        &self,
        unit: &ScopePath,
        seqs: &BTreeMap<ScopePath, Seq>,
    ) -> impl Future<Output = Result<UnitCut, WpsError>> + Send;
    fn read(&self, scope: &ScopePath, at: Seq) -> impl Future<Output = Result<ScopeSnapshot, WpsError>> + Send;
    fn read_unit(&self, cut: &UnitCut) -> impl Future<Output = Result<UnitSnapshot, WpsError>> + Send;
    fn entity(
        &self,
        scope: &ScopePath,
        id: &EntityId,
        at: Seq,
    ) -> impl Future<Output = Result<EntityReading, WpsError>> + Send;
    fn derived(
        &self,
        scope: &ScopePath,
        at: Seq,
        under: Option<&EntityId>,
        after: Option<String>,
    ) -> impl Future<Output = Result<Page<DerivedListing>, WpsError>> + Send;
    fn referrers(
        &self,
        scope: &ScopePath,
        id: &EntityId,
        at: Seq,
        after: Option<String>,
    ) -> impl Future<Output = Result<Page<Referrer>, WpsError>> + Send;
    fn history(
        &self,
        scope: &ScopePath,
        before: Option<Seq>,
        limit: u16,
    ) -> impl Future<Output = Result<Vec<LogEntry>, WpsError>> + Send;
    /// The cut's site as WPS generated it with every entity it references, its flagged export, closed and written as a
    /// run reads it.
    fn export(&self, selection: &UnitCut) -> impl Future<Output = Result<CutExport, WpsError>> + Send;
    fn check(
        &self,
        draft: &ChangesetDraft,
        base: &UnitSnapshot,
    ) -> impl Future<Output = Result<Vec<Problem>, WpsError>> + Send;
    fn preview(
        &self,
        draft: &ChangesetDraft,
        base: &UnitSnapshot,
    ) -> impl Future<Output = Result<Preview, WpsError>> + Send;
    fn apply(&self, draft: &ChangesetDraft) -> impl Future<Output = Result<Applied, WpsError>> + Send;
    /// A write whose outcome was left unknown, as by a restart, settled by sending `sent` again under its identifier:
    /// the body `apply::changeset_of` made of `draft`, which its caller kept before applying it. WPS answers the first
    /// sending's whole answer, replayed, when it landed, and applies or refuses it as a first sending when it did not;
    /// only that refusal, its refusal object in hand, is `NotLanded::Refused`. A body other than the one the draft
    /// makes is never sent, its outcome left as unknown as before.
    fn resend(
        &self,
        sent: &WireChangeset,
        draft: &ChangesetDraft,
    ) -> impl Future<Output = Result<Applied, NotLanded>> + Send;
    fn propose_import(
        &self,
        scope: &ScopePath,
        base: Seq,
        document: &Value,
    ) -> impl Future<Output = Result<ImportProposal, WpsError>> + Send;
    fn tag(
        &self,
        scope: &ScopePath,
        name: &TagName,
        at: &Pin,
        author: &Author,
    ) -> impl Future<Output = Result<UnitCut, WpsError>> + Send;
    /// `scope`'s moves from the head it stands at once the watch is ready.
    fn events(&self, scope: &ScopePath) -> ScopeEvents;
    /// `scope`'s moves past `known`, the changeset a consumer last knew it at, such as a draft's base: a head already
    /// past it when the watch is ready is told first as `Resynced`, so no move before the subscription goes unheard.
    fn events_since(&self, scope: &ScopePath, known: Seq) -> ScopeEvents;
    fn registry(&self) -> &SchemaCache;
    fn floors(&self) -> &FloorCache;
}

/// What the client is told besides the server: the revision dev.sh built, what the schema cache preloads, the
/// caches' budgets, and the wall-clock patience of its refreshing and reconnecting.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub revision: Option<String>,
    pub preload: Vec<String>,
    pub export_budget: usize,
    pub floor_budget: usize,
    pub revalidate_every: Duration,
    pub reconnect_first: Duration,
    pub reconnect_last: Duration,
    /// How many events a slow consumer may fall behind before its next is marked as a gap.
    pub event_backlog: usize,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            revision: None,
            preload: PRELOAD.iter().map(|name| (*name).to_owned()).collect(),
            export_budget: DEFAULT_BUDGET,
            floor_budget: DEFAULT_BUDGET,
            revalidate_every: Duration::from_secs(60),
            reconnect_first: Duration::from_secs(1),
            reconnect_last: Duration::from_secs(30),
            event_backlog: 1024,
        }
    }
}

/// `STUDIO_WPS_URL`, `STUDIO_WPS_ROUTES` and `STUDIO_WPS_REVISION` as the seed and the runner read them; the API
/// reads its own configuration and hands the same values over.
pub fn http_from_env() -> Result<(HttpConfig, ClientConfig), String> {
    let stated = |var: &str| env::var(var).ok().filter(|value| !value.trim().is_empty());
    let url = stated(URL_VAR).unwrap_or_else(|| DEFAULT_URL.to_owned());
    let base = Url::parse(&url).map_err(|fault| format!("{URL_VAR}={url}: {fault}"))?;
    let shape: RouteShape = stated(ROUTES_VAR).map_or(Ok(RouteShape::Path), |text| text.parse())?;
    let routes = Routes::new(base, shape)?;
    let config = ClientConfig { revision: stated(REVISION_VAR), ..ClientConfig::default() };
    Ok((HttpConfig::new(routes), config))
}

/// The one implementation of `WorldStore`, over a real server or the mock.
pub struct Client<S: Wps> {
    pub(crate) wps: Arc<S>,
    pub(crate) config: ClientConfig,
    pub(crate) capabilities: Capabilities,
    pub(crate) schemas: Arc<SchemaCache>,
    pub(crate) exports: ExportCache,
    pub(crate) floors: FloorCache,
    pub(crate) compute: Arc<dyn Compute>,
    pub(crate) hub: EventHub<S>,
}

impl Client<HttpWps> {
    /// A client of the server the environment names.
    pub async fn from_env(compute: Arc<dyn Compute>) -> Result<Self, WpsError> {
        let (http, config) = http_from_env().map_err(|text| WpsError::Incompatible { text })?;
        Self::start(HttpWps::new(http)?, config, compute).await
    }
}

impl<S: Wps> Client<S> {
    /// Probe the server, writing nothing, and load the schema cache (spec 04 section 3.12, then 3.10).
    pub async fn start(wps: S, config: ClientConfig, compute: Arc<dyn Compute>) -> Result<Self, WpsError> {
        let wps = Arc::new(wps);
        let capabilities = probe(&*wps, config.revision.as_deref()).await?;
        let schemas = Arc::new(SchemaCache::default());
        let preload: Vec<&str> = config.preload.iter().map(String::as_str).collect();
        schemas.load(&*wps, &capabilities.proto_version, &preload).await?;
        let hub = EventHub::new(wps.clone(), config.reconnect_first, config.reconnect_last, config.event_backlog);
        Ok(Self {
            exports: export_cache(config.export_budget),
            floors: floor_cache(config.floor_budget),
            wps,
            config,
            capabilities,
            schemas,
            compute,
            hub,
        })
    }

    /// Revalidate the schema cache every `revalidate_every` of wall clock, for as long as the client lives.
    pub fn keep_schemas_fresh(self: &Arc<Self>) {
        let client = Arc::downgrade(self);
        let every = self.config.revalidate_every;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                let Some(client) = client.upgrade() else { return };
                let _ = client.schemas.revalidate(&*client.wps).await;
            }
        });
    }

    /// The server this client speaks to, for a test to inspect.
    pub fn wps(&self) -> &S {
        &self.wps
    }

    pub fn capabilities_now(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn exports(&self) -> &ExportCache {
        &self.exports
    }

    /// One read of the scope tree: every scope's flag, unit and head, in one statement.
    pub async fn tree(&self) -> Result<ScopeTree, WpsError> {
        tree_of(&*self.wps).await
    }

    /// Every scope at or below `under`, in path order, from one statement: `GET /scopes?under=`.
    async fn listed_under(&self, under: &ScopePath) -> Result<Vec<ScopeListing>, WpsError> {
        let rows = self.wps.scopes_under(under).await?;
        let mut listing = Vec::new();
        for row in rows {
            listing.extend(row.listing()?.filter(|scope| under.holds(&scope.path)));
        }
        listing.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(listing)
    }

    /// The subtree at `under` as one read found it: each scope's flag, unit and head, the heads standing together.
    pub async fn subtree(&self, under: &ScopePath) -> Result<ScopeTree, WpsError> {
        Ok(ScopeTree::new(self.listed_under(under).await?))
    }

    /// The scopes of the unit `anchor` stands in, at one read: the anchor's subtree when it is its unit's root, else
    /// the root's, read second.
    async fn unit_tree(&self, anchor: &ScopePath) -> Result<(ScopePath, ScopeTree), WpsError> {
        let tree = self.subtree(anchor).await?;
        let unit = Self::listed(&tree, anchor)?.unit.clone();
        match unit == *anchor {
            true => Ok((unit, tree)),
            false => Ok((unit.clone(), self.subtree(&unit).await?)),
        }
    }

    fn listed<'a>(tree: &'a ScopeTree, scope: &ScopePath) -> Result<&'a ScopeListing, WpsError> {
        tree.get(scope).ok_or_else(|| WpsError::NotFound { text: format!("there is no scope {scope}") })
    }

    pub(crate) async fn read_at(
        &self,
        scope: &ScopePath,
        at: Seq,
        exact: Exactness,
    ) -> Result<ScopeSnapshot, WpsError> {
        let raw = self.wps.export_scope(scope, Some(at)).await?;
        let doc: Value = serde_json::from_slice(&raw.bytes)
            .map_err(|fault| WpsError::Invalid { text: format!("{scope}'s export is not JSON: {fault}") })?;
        snapshot_of(scope, raw.base, exact, &doc)
    }
}

async fn tree_of<S: Wps>(wps: &S) -> Result<ScopeTree, WpsError> {
    let mut listing = Vec::new();
    for row in wps.scopes().await? {
        listing.extend(row.listing()?);
    }
    Ok(ScopeTree::new(listing))
}

/// One scope's export read as its authored form.
pub fn snapshot_of(scope: &ScopePath, at: Seq, exact: Exactness, doc: &Value) -> Result<ScopeSnapshot, WpsError> {
    let invalid = |what: &str| WpsError::Invalid { text: format!("{scope}'s export at {at}: {what}") };
    let prologue = match doc.pointer("/doc/id").and_then(Value::as_str) {
        None => None,
        Some(id) => Some(Prologue {
            doc: id.parse().map_err(|_| invalid("its document id is not a UUID"))?,
            kind: doc.pointer("/doc/kind").and_then(Value::as_str).map(str::to_owned),
            name: doc.pointer("/doc/name").and_then(Value::as_str).map(str::to_owned),
            primary_frame: doc.get("primary_frame").and_then(Value::as_str).and_then(|frame| frame.parse().ok()),
            depends_on: doc
                .pointer("/doc/depends_on")
                .and_then(Value::as_array)
                .map(|depends| depends.iter().filter_map(|id| id.as_str()?.parse().ok()).collect())
                .unwrap_or_default(),
        }),
    };
    let (mut entities, mut records) = (Vec::new(), Vec::new());
    for entry in doc.get("entities").and_then(Value::as_array).into_iter().flatten() {
        let components = entry.get("components").and_then(Value::as_object).cloned().unwrap_or_default();
        if let Some(derivation) = entry.get("derivation") {
            let generator = derivation.get("generator").and_then(Value::as_str).unwrap_or_default();
            let key = derivation.get("key").and_then(Value::as_str).unwrap_or_default();
            let target = format!("{generator}/{key}").parse().map_err(|_| invalid("a record's derivation"))?;
            records.push(Record::Override { target, components });
            continue;
        }
        let id = entry.get("id").and_then(Value::as_str).ok_or_else(|| invalid("an entry has no id"))?;
        if entry.get("suppress").and_then(Value::as_bool) == Some(true) {
            records.push(Record::Suppress { target: id.parse().map_err(|_| invalid("a suppression's target"))? });
            continue;
        }
        let id: DeclaredId = id.parse().map_err(|_| invalid("a declared id is not a UUID"))?;
        let name = entry.get("name").and_then(Value::as_str).map(str::to_owned);
        entities.push(DeclaredEntity { id, name, components });
    }
    Ok(ScopeSnapshot { scope: scope.clone(), at, exact, prologue, entities, records })
}

impl<S: Wps> WorldStore for Client<S> {
    async fn capabilities(&self) -> Result<Capabilities, WpsError> {
        Ok(self.capabilities.clone())
    }

    async fn list_scopes(&self, under: &ScopePath) -> Result<Vec<ScopeListing>, WpsError> {
        self.listed_under(under).await
    }

    async fn ensure_scope(&self, path: &ScopePath, boundary: bool) -> Result<ScopeListing, WpsError> {
        self.ensure(path, boundary).await
    }

    async fn resolve_pin(&self, scope: &ScopePath, pin: &Pin) -> Result<Seq, WpsError> {
        let tree = self.subtree(scope).await?;
        let head = Self::listed(&tree, scope)?.head;
        match pin {
            Pin::Changeset(seq) if *seq <= head => Ok(*seq),
            Pin::Changeset(seq) => Err(WpsError::PinUnresolved {
                scope: scope.clone(),
                text: format!("{scope} has no changeset {seq}; its newest on main is {head}"),
            }),
            Pin::Branch(branch) if *branch == BranchName::main() => Ok(head),
            Pin::Branch(_) => Err(WpsError::Unsupported { capability: Capability::Branches }),
            Pin::Tag(_) => Err(WpsError::Unsupported { capability: Capability::Tags }),
        }
    }

    async fn resolve_cut(&self, anchor: &ScopePath, pin: &Pin) -> Result<UnitCut, WpsError> {
        let (unit, tree) = self.unit_tree(anchor).await?;
        let heads: BTreeMap<ScopePath, Seq> =
            tree.unit(&unit).into_iter().map(|scope| (scope.path.clone(), scope.head)).collect();
        let seqs = match pin {
            Pin::Branch(branch) if *branch == BranchName::main() => heads.clone(),
            Pin::Branch(_) => return Err(WpsError::Unsupported { capability: Capability::Branches }),
            Pin::Tag(_) => return Err(WpsError::Unsupported { capability: Capability::Tags }),
            // Placing a site's other scopes at one changeset of the anchor takes WPS's commit places, which phase 5
            // serves and no route answers yet; a wall-clock timestamp is no commit order, so a multi-scope site at a
            // changeset is refused and its caller names each scope's changeset as a fixed cut.
            Pin::Changeset(_) if heads.len() > 1 => {
                return Err(WpsError::Unsupported { capability: Capability::ReadsAtVersion });
            }
            Pin::Changeset(_) => BTreeMap::from([(anchor.clone(), self.resolve_pin(anchor, pin).await?)]),
        };
        let mut exact = Exactness::Exact;
        for (scope, seq) in &seqs {
            if heads.get(scope).is_some_and(|head| head > seq) {
                exact = exact.and(Exactness::below_head(scope, *seq));
            }
        }
        Ok(UnitCut { unit, requested: pin.clone(), branch: BranchName::main(), seqs, exact })
    }

    async fn resolve_fixed_cut(&self, unit: &ScopePath, seqs: &BTreeMap<ScopePath, Seq>) -> Result<UnitCut, WpsError> {
        let tree = self.subtree(unit).await?;
        let scopes = tree.unit(unit);
        let unresolved = |scope: &ScopePath, text: String| WpsError::PinUnresolved { scope: scope.clone(), text };
        let mut exact = Exactness::Exact;
        for listing in &scopes {
            let Some(seq) = seqs.get(&listing.path) else {
                return Err(unresolved(&listing.path, format!("the fixed cut leaves out {}", listing.path)));
            };
            if *seq > listing.head {
                let text = format!("{} has no changeset {seq}; its newest on main is {}", listing.path, listing.head);
                return Err(unresolved(&listing.path, text));
            }
            if *seq < listing.head {
                exact = exact.and(Exactness::below_head(&listing.path, *seq));
            }
        }
        if let Some(stray) = seqs.keys().find(|scope| !scopes.iter().any(|listing| &listing.path == *scope)) {
            return Err(unresolved(stray, format!("{stray} is no scope of {unit}")));
        }
        let requested = Pin::Branch(BranchName::main());
        Ok(UnitCut { unit: unit.clone(), requested, branch: BranchName::main(), seqs: seqs.clone(), exact })
    }

    async fn read(&self, scope: &ScopePath, at: Seq) -> Result<ScopeSnapshot, WpsError> {
        let tree = self.subtree(scope).await?;
        let head = Self::listed(&tree, scope)?.head;
        let exact = if at == head { Exactness::Exact } else { Exactness::below_head(scope, at) };
        self.read_at(scope, at, exact).await
    }

    async fn read_unit(&self, cut: &UnitCut) -> Result<UnitSnapshot, WpsError> {
        let mut scopes = BTreeMap::new();
        for (scope, seq) in &cut.seqs {
            scopes.insert(scope.clone(), self.read_at(scope, *seq, cut.exact.clone()).await?);
        }
        Ok(UnitSnapshot::new(cut.clone(), scopes))
    }

    async fn entity(&self, scope: &ScopePath, id: &EntityId, at: Seq) -> Result<EntityReading, WpsError> {
        Ok(self.wps.entity(scope, id, Some(at)).await?.into())
    }

    async fn derived(
        &self,
        scope: &ScopePath,
        at: Seq,
        under: Option<&EntityId>,
        after: Option<String>,
    ) -> Result<Page<DerivedListing>, WpsError> {
        let query = EntityQuery {
            at: Some(at),
            under: under.cloned(),
            provenance: Some("derived".to_owned()),
            after,
            limit: None,
        };
        let (items, next) = self.wps.entities(scope, &query).await?;
        Ok(Page { items: items.into_iter().map(Into::into).collect(), next })
    }

    async fn referrers(
        &self,
        scope: &ScopePath,
        id: &EntityId,
        at: Seq,
        after: Option<String>,
    ) -> Result<Page<Referrer>, WpsError> {
        let (items, next) = self.wps.referrers(scope, id, Some(at), after).await?;
        Ok(Page { items: items.into_iter().map(Into::into).collect(), next })
    }

    async fn history(&self, scope: &ScopePath, before: Option<Seq>, limit: u16) -> Result<Vec<LogEntry>, WpsError> {
        self.wps.changesets(scope, before, limit).await
    }

    async fn export(&self, selection: &UnitCut) -> Result<CutExport, WpsError> {
        if !self.capabilities.has(Capability::ExportFlags) {
            return Err(WpsError::Unsupported { capability: Capability::ExportFlags });
        }
        let key = ExportKey { seqs: selection.seqs.clone() };
        let (wps, pool, schemas, cut) =
            (self.wps.clone(), self.compute.clone(), self.schemas.clone(), selection.clone());
        let export = self
            .exports
            .get_or_spawn(key, move || async move {
                let tree = tree_of(&*wps).await?;
                Ok::<_, WpsError>(Arc::new(flagged(&*wps, &*pool, &schemas, &tree, &cut).await?))
            })
            .await?;
        Ok((*export).clone())
    }

    async fn check(&self, draft: &ChangesetDraft, base: &UnitSnapshot) -> Result<Vec<Problem>, WpsError> {
        self.check_draft(draft, base).await
    }

    async fn preview(&self, draft: &ChangesetDraft, base: &UnitSnapshot) -> Result<Preview, WpsError> {
        // WPS's preview (phase 9) has no route yet, so nothing is sent: the client's own check answers it.
        let problems = self.check_draft(draft, base).await?;
        Ok(Preview { checked_by: CheckedBy::AgentStudio, problems })
    }

    async fn apply(&self, draft: &ChangesetDraft) -> Result<Applied, WpsError> {
        self.apply_draft(draft).await
    }

    async fn resend(&self, sent: &WireChangeset, draft: &ChangesetDraft) -> Result<Applied, NotLanded> {
        self.resend_draft(sent, draft).await
    }

    async fn propose_import(&self, scope: &ScopePath, base: Seq, document: &Value) -> Result<ImportProposal, WpsError> {
        let proposal = self.wps.import(scope, base, document).await?;
        let changeset = proposal.changeset;
        let mut prologues = BTreeMap::new();
        if let Some(prologue) = changeset.prologue {
            prologues.insert(scope.clone(), prologue);
        }
        let draft = ChangesetDraft {
            initiated_on: scope.clone(),
            bases: BTreeMap::from([(scope.clone(), base)]),
            creates: Vec::new(),
            prologues,
            author: Author::new("import").expect("a word"),
            message: changeset.message,
            ops: changeset
                .ops
                .into_iter()
                .map(|wire| ScopedOp { scope: wire.scope.unwrap_or_else(|| scope.clone()), op: wire.op })
                .collect(),
            expect_generators: None,
            // The caller names it before applying the proposal: a write without one is refused, nothing sent.
            operation: String::new(),
            minted: Vec::new(),
        };
        Ok(ImportProposal { draft, tally: proposal.tally, warnings: proposal.warnings })
    }

    async fn tag(&self, _scope: &ScopePath, _name: &TagName, _at: &Pin, _author: &Author) -> Result<UnitCut, WpsError> {
        Err(WpsError::Unsupported { capability: Capability::Tags })
    }

    fn events(&self, scope: &ScopePath) -> ScopeEvents {
        self.hub.watch(scope, None)
    }

    fn events_since(&self, scope: &ScopePath, known: Seq) -> ScopeEvents {
        self.hub.watch(scope, Some(known))
    }

    fn registry(&self) -> &SchemaCache {
        &self.schemas
    }

    fn floors(&self) -> &FloorCache {
        &self.floors
    }
}
