//! Within `wps-client`, `site` is the scope side of a site's life (spec 04 sections 3.4 and 3.9): `ensure_scope` and
//! the prefixes the seed makes with their boundary flags; Create site (spec 05 section 4.4), which makes a site's
//! root, its `map`, `flow` and `fleet` and any missing customer prefix, and writes all four with their prologues, in
//! ONE changeset WPS applies whole or refuses whole, its body kept by the caller before it is sent so that a lost
//! answer is settled by WPS's replay of it; and the site list `GET /api/sites` reads (spec 01 section 3.6).

use std::cmp::Reverse;
use std::collections::BTreeMap;

use chrono::DateTime;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use studio_protocol::{BranchName, DeclaredId, Pin, ScopePath, Seq};

use crate::apply::{NotLanded, changeset_of};
use crate::client::{Client, WorldStore};
use crate::draft::{Applied, ChangesetDraft, NewScope, Op, ScopedOp};
use crate::error::WpsError;
use crate::kinship::{ScopeTree, ancestry};
use crate::snapshot::{ScopeSnapshot, UnitSnapshot};
use crate::value::{Author, DocId, Exactness, GeneratorSet, Prologue, ScopeListing, UnitCut};
use crate::wire::{EntityQuery, WireChangeset};
use crate::wps::Wps;

/// The sub-scopes of a site's or a template's root (ruling D107).
pub const SUB_SCOPES: [&str; 3] = ["map", "flow", "fleet"];

/// Whether `scope` is a root the client flags as a boundary: `customer/{c}/site/{s}`, `template/site/{t}` or
/// `catalog/{kind}/{item}` (spec 04 section 3.1).
pub fn is_root(scope: &ScopePath) -> bool {
    let slugs: Vec<&str> = scope.slugs().collect();
    matches!(slugs.as_slice(), ["customer", _, "site", _] | ["template", "site", _] | ["catalog", _, _])
}

/// Whether `scope` is a site's or a template's root, whose export dessim and the engine load whole and which so
/// names its primary frame.
pub fn is_loadable_root(scope: &ScopePath) -> bool {
    let slugs: Vec<&str> = scope.slugs().collect();
    matches!(slugs.as_slice(), ["customer", _, "site", _] | ["template", "site", _])
}

impl<S: Wps> Client<S> {
    /// Create `path` with `boundary`, or reuse it when it exists with that flag and holds no live entity.
    pub(crate) async fn ensure(&self, path: &ScopePath, boundary: bool) -> Result<ScopeListing, WpsError> {
        match self.wps.create_scope(path, boundary).await {
            Ok(row) => row.listing()?.ok_or_else(|| WpsError::Invalid { text: format!("{path} was created flagless") }),
            Err(WpsError::ScopeExists { .. }) => {
                let tree = self.subtree(path).await?;
                let held =
                    tree.get(path).cloned().ok_or_else(|| WpsError::NotFound { text: format!("no scope {path}") })?;
                let query = EntityQuery { limit: Some(1), ..EntityQuery::default() };
                let (live, _) = self.wps.entities(path, &query).await?;
                if held.boundary == boundary && live.is_empty() {
                    Ok(held)
                } else {
                    Err(WpsError::ScopeExists { scope: path.clone(), head: held.head })
                }
            }
            Err(other) => Err(other),
        }
    }
}

/// Make every missing proper prefix of `path`, shortest first, with the flag spec 04 section 3.1 gives it, and hold
/// each existing one to that flag only: an ancestor may hold entities (a template's root under its `map`), and only
/// the scope being made must be empty to be reused.
pub async fn ensure_prefixes<W: WorldStore>(store: &W, path: &ScopePath) -> Result<Vec<ScopeListing>, WpsError> {
    let prefixes: Vec<ScopePath> = ancestry(path).into_iter().skip(1).rev().collect();
    let Some(top) = prefixes.first() else { return Ok(Vec::new()) };
    let existing: FxHashMap<ScopePath, ScopeListing> =
        store.list_scopes(top).await?.into_iter().map(|listing| (listing.path.clone(), listing)).collect();
    let mut made = Vec::new();
    for prefix in prefixes {
        let boundary = is_root(&prefix);
        made.push(match existing.get(&prefix) {
            Some(listing) if listing.boundary == boundary => listing.clone(),
            Some(listing) => return Err(WpsError::ScopeExists { scope: prefix, head: listing.head }),
            None => store.ensure_scope(&prefix, boundary).await?,
        });
    }
    Ok(made)
}

/// What Create site writes (spec 05 section 4.4): a site's root and its `map`, `flow` and `fleet`, each stating its
/// prologue, with the entities the evaluation declares in each under the ids it minted and its records, one author,
/// message and operation identifier, and the generator set the evaluation ran. It holds nothing read from WPS, the
/// prefixes to create being read when its body is built (`site_changeset`), and serializes, so the API keeps it with
/// the approval beside that body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SitePlan {
    pub root: ScopePath,
    // ordered: one plan makes one body, its scopes in path order.
    pub prologues: BTreeMap<ScopePath, Prologue>,
    pub ops: Vec<ScopedOp>,
    pub author: Author,
    pub message: String,
    pub operation: String,
    pub expect_generators: Option<GeneratorSet>,
}

impl SitePlan {
    /// A plan for the site at `root`, `customer/{c}/site/{s}`, whose root states `prologue`, the evaluation's with
    /// the name the person typed: each sub-scope states a document of its own, minted here once and named
    /// "{name}: {sub-scope}" as the seed names a template's.
    pub fn new(
        root: ScopePath,
        prologue: Prologue,
        author: Author,
        message: impl Into<String>,
        operation: impl Into<String>,
    ) -> Result<Self, WpsError> {
        if !matches!(root.slugs().collect::<Vec<_>>().as_slice(), ["customer", _, "site", _]) {
            return Err(WpsError::Invalid { text: format!("Create site makes customer/{{c}}/site/{{s}}, not {root}") });
        }
        let mut prologues = BTreeMap::new();
        for sub in SUB_SCOPES {
            let name = prologue.name.as_ref().map(|name| format!("{name}: {sub}"));
            let document = Prologue {
                doc: DocId::mint(),
                kind: Some("world".to_owned()),
                name,
                primary_frame: None,
                depends_on: Vec::new(),
            };
            prologues.insert(sub_scope(&root, sub)?, document);
        }
        prologues.insert(root.clone(), prologue);
        let (message, operation) = (message.into(), operation.into());
        Ok(Self { root, prologues, ops: Vec::new(), author, message, operation, expect_generators: None })
    }

    /// The four scopes the site's changeset writes: its root and the root's `map`, `flow` and `fleet`.
    pub fn scopes(&self) -> Vec<ScopePath> {
        self.prologues.keys().cloned().collect()
    }

    fn holds(&self, scope: &ScopePath) -> Result<(), WpsError> {
        match self.prologues.contains_key(scope) {
            true => Ok(()),
            false => Err(WpsError::Invalid { text: format!("{scope} is not one of the scopes of {}", self.root) }),
        }
    }

    /// Declare an entity the evaluation made in `scope`, under the id it minted for it.
    pub fn declare(
        &mut self,
        scope: &ScopePath,
        id: DeclaredId,
        name: Option<String>,
        components: Map<String, Value>,
    ) -> Result<(), WpsError> {
        self.holds(scope)?;
        self.ops.push(ScopedOp { scope: scope.clone(), op: Op::Declare { id, name, components } });
        Ok(())
    }

    /// A record of the evaluation's in `scope`: an override or a suppression of an address WPS derives.
    pub fn record(&mut self, scope: &ScopePath, record: Op) -> Result<(), WpsError> {
        self.holds(scope)?;
        if !matches!(record, Op::Override { .. } | Op::Suppress { .. }) {
            return Err(WpsError::Invalid { text: "a made site's record is an override or a suppression".to_owned() });
        }
        self.ops.push(ScopedOp { scope: scope.clone(), op: record });
        Ok(())
    }

    pub fn expect_generators(&mut self, generators: GeneratorSet) {
        self.expect_generators = Some(generators);
    }

    /// The one changeset that makes the site over `tree`, the scopes at and below `customer` as one read found them:
    /// it creates each missing prefix flagless, the root a boundary and its sub-scopes flagless, is initiated on the
    /// first scope it creates, so every scope it writes or creates is that one or below it (ruling D72), and writes
    /// each of the four scopes from base 0. A root that exists, or a prefix with the other flag, is `ScopeExists`.
    pub fn draft(&self, tree: &ScopeTree) -> Result<ChangesetDraft, WpsError> {
        if let Some(held) = tree.get(&self.root) {
            return Err(WpsError::ScopeExists { scope: self.root.clone(), head: held.head });
        }
        let mut creates = Vec::new();
        for prefix in ancestry(&self.root).into_iter().skip(1) {
            match tree.get(&prefix) {
                Some(held) if held.boundary => return Err(WpsError::ScopeExists { scope: prefix, head: held.head }),
                Some(_) => {}
                None => creates.push(NewScope { path: prefix, boundary: false }),
            }
        }
        Ok(self.changeset(creates))
    }

    /// The site's changeset as though every prefix of its root stood: its four scopes created, nothing else, which
    /// `check` holds against `empty_base` before the site's path is chosen (spec 05 section 4.3, step 6.12).
    pub fn site_alone(&self) -> ChangesetDraft {
        self.changeset(Vec::new())
    }

    /// The changeset creating `creates` beside the site's four scopes, initiated on the first in path order.
    fn changeset(&self, mut creates: Vec<NewScope>) -> ChangesetDraft {
        creates.extend(self.scopes().into_iter().map(|path| NewScope { boundary: path == self.root, path }));
        creates.sort_by(|a, b| a.path.cmp(&b.path));
        ChangesetDraft {
            initiated_on: creates[0].path.clone(),
            bases: creates.iter().map(|new| (new.path.clone(), Seq::new(0))).collect(),
            prologues: self.prologues.clone(),
            author: self.author.clone(),
            message: self.message.clone(),
            ops: self.ops.clone(),
            expect_generators: self.expect_generators.clone(),
            operation: self.operation.clone(),
            minted: self
                .ops
                .iter()
                .filter_map(|scoped| match &scoped.op {
                    Op::Declare { id, .. } => Some(*id),
                    _ => None,
                })
                .collect(),
            creates,
        }
    }

    /// The customer prefixes `sent` creates, each flagless as `draft` makes it: with them, the plan's own changeset is
    /// the one `sent` should be.
    fn prefixes_in(&self, sent: &WireChangeset) -> Vec<NewScope> {
        let prefixes = sent.create.iter().filter(|new| new.path != self.root && new.path.holds(&self.root));
        prefixes.map(|new| NewScope { path: new.path.clone(), boundary: false }).collect()
    }

    /// The site's four scopes, empty at changeset 0: the base an evaluation's `check` holds the would-be changeset
    /// against before the site exists (spec 05 section 4.3).
    pub fn empty_base(&self) -> UnitSnapshot {
        let empty = |scope: &ScopePath| ScopeSnapshot {
            scope: scope.clone(),
            at: Seq::new(0),
            exact: Exactness::Exact,
            prologue: None,
            entities: Vec::new(),
            records: Vec::new(),
        };
        let cut = UnitCut {
            unit: self.root.clone(),
            requested: Pin::Branch(BranchName::main()),
            branch: BranchName::main(),
            seqs: self.scopes().into_iter().map(|scope| (scope, Seq::new(0))).collect(),
            exact: Exactness::Exact,
        };
        UnitSnapshot::new(cut, self.scopes().iter().map(|scope| (scope.clone(), empty(scope))).collect())
    }
}

fn sub_scope(root: &ScopePath, sub: &str) -> Result<ScopePath, WpsError> {
    root.join(&sub.parse().expect("a slug")).map_err(|fault| WpsError::Invalid { text: fault.to_string() })
}

/// Create site's body: the plan's one changeset over one read of the scopes at and below `customer`, which the caller
/// keeps with the approval before `create_site` sends it. A root that exists is `ScopeExists`.
pub async fn site_changeset<S: Wps>(client: &Client<S>, plan: &SitePlan) -> Result<WireChangeset, WpsError> {
    let customer: ScopePath = "customer".parse().expect("a slug");
    changeset_of(&plan.draft(&client.subtree(&customer).await?)?)
}

/// Create site: `sent`, the body `site_changeset` built for `plan` and the caller kept, held to the plan and sent as it
/// was under its operation identifier, reading nothing first (LEAD-DECISIONS entry 54); each scope stands at its first
/// changeset in the answer. Sent again, as after the API restarted, it is answered the first sending's whole answer,
/// replayed, writing nothing. `NotLanded::Refused` is WPS's own refusal, which stored nothing under the identifier: a
/// scope it creates that exists is `ScopeExists` naming the first such scope, after which a body built again
/// (`site_changeset`) and kept in place of this one makes the site past a prefix another writer made, or finds the
/// root standing, spec 05's `site-exists`. A body the plan does not make is never sent, and since an earlier sending
/// of it may have landed, it is `NotLanded::Unknown` like a lost answer.
pub async fn create_site<S: Wps>(
    client: &Client<S>,
    plan: &SitePlan,
    sent: &WireChangeset,
) -> Result<Applied, NotLanded> {
    let draft = plan.changeset(plan.prefixes_in(sent));
    if changeset_of(&draft).map_err(NotLanded::Unknown)? != *sent {
        let text = format!(
            "the body kept for Create site of {} is not the changeset its plan makes, so nothing was sent",
            plan.root
        );
        return Err(NotLanded::Unknown(WpsError::Invalid { text }));
    }
    client.post(&draft, sent).await
}

/// One site of the site list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SiteRow {
    pub root: ScopePath,
    /// The root's prologue name.
    pub name: Option<String>,
    /// The newest `applied_at` among the site's scope heads (RFC 3339, wall clock).
    pub changed_at: String,
    // ordered: a site's scopes are shown in path order.
    pub heads: BTreeMap<ScopePath, Seq>,
}

/// Every `customer/*/site/*` root whose head is above 0, of one customer when `customer` names it, each named by its
/// root's prologue, newest change first, from one read of the scopes beneath. Create site writes the root in the one
/// changeset that makes the site, so a root standing at 0 was made by name and never written, and is no site.
pub async fn list_sites<S: Wps>(client: &Client<S>, customer: Option<&str>) -> Result<Vec<SiteRow>, WpsError> {
    let under = match customer {
        None => "customer".to_owned(),
        Some(customer) => format!("customer/{customer}"),
    };
    let under: ScopePath = under.parse().map_err(|fault| WpsError::Invalid { text: format!("{under}: {fault}") })?;
    let listing = client.list_scopes(&under).await?;
    let tree = ScopeTree::new(listing.clone());
    let mut rows = Vec::new();
    for root in listing {
        let slugs: Vec<&str> = root.path.slugs().collect();
        if !matches!(slugs.as_slice(), ["customer", _, "site", _]) || !root.boundary || root.head == Seq::new(0) {
            continue;
        }
        let mut heads = BTreeMap::new();
        let mut changed_at: Option<String> = None;
        for scope in tree.unit(&root.path) {
            heads.insert(scope.path.clone(), scope.head);
            if scope.head == Seq::new(0) {
                continue;
            }
            let entry = client.history(&scope.path, None, 1).await?;
            if let Some(entry) = entry.first() {
                let newer = changed_at.as_deref().is_none_or(|known| later(&entry.applied_at, known));
                if newer {
                    changed_at = Some(entry.applied_at.clone());
                }
            }
        }
        let name = site_name(client, &root).await?;
        rows.push(SiteRow { root: root.path.clone(), name, changed_at: changed_at.unwrap_or_default(), heads });
    }
    rows.sort_by_cached_key(|row| (Reverse(DateTime::parse_from_rfc3339(&row.changed_at).ok()), row.root.clone()));
    Ok(rows)
}

/// A site's name, as the site list and the Map panel say it: its root's listed name, else the name its root's prologue
/// states at the root's head, which a read below the head carries too, until exact history.
pub async fn site_name<S: Wps>(client: &Client<S>, root: &ScopeListing) -> Result<Option<String>, WpsError> {
    match &root.name {
        Some(name) => Ok(Some(name.clone())),
        None => Ok(client.read(&root.path, root.head).await?.prologue.and_then(|prologue| prologue.name)),
    }
}

fn later(a: &str, b: &str) -> bool {
    match (DateTime::parse_from_rfc3339(a), DateTime::parse_from_rfc3339(b)) {
        (Ok(a), Ok(b)) => a > b,
        _ => a > b,
    }
}
