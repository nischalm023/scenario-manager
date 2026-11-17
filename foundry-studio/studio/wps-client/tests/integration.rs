//! Spec 04 section 8's integration statements 10 to 14 and 18, spec 01 statement 2's second half, and the gates the
//! mock leaves to WPS (a deletion still named, a reference to what its changeset deletes, a pin cycle, a registry
//! family's read base), against the world-proto-server `STUDIO_TEST_WPS_URL` names: the one dev.sh starts from `WORLD_PROTO_DIR`, seeded by its
//! own demo seed, master 092d695 or later, because every write needs scopes phase 3. Each test is skipped, and says so,
//! without its variables; statement 18's two instances need `STUDIO_TEST_WPS_SECOND_URL`. `STUDIO_WPS_REVISION` names
//! the revision as dev.sh would.

mod support;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::Client as RawHttp;
use semver::Version as SemVer;
use serde_json::{Map, Value, json};
use site_model::flagged_export;
use studio_protocol::{BranchName, DeclaredId, EntityId, Pin, ScopePath, Seq};
use support::{object, path};
use tokio::time::timeout;
use url::Url;
use wps_client::apply::{NotLanded, changeset_of};
use wps_client::refusal::{Unresolved, unresolved};
use wps_client::routes::{RouteShape, Routes};
use wps_client::seed::{self, SeedError};
use wps_client::site::{SUB_SCOPES, SitePlan, create_site, ensure_prefixes, list_sites, site_changeset};
use wps_client::walker::OWN_LIST;
use wps_client::wire::{
    EntityQuery, ExportFlags, RawExport, Resolution, WireApplied, WireChangeset, WireComponent, WireDefinition,
    WireEntity, WireGenerator, WireGenerators, WireImport, WireListed, WireProtocol, WireReferrer, WireScope,
    WireTrail,
};
use wps_client::wps::{EventStream, PostFailure, Wps};
use wps_client::{
    Applied, Author, BlockingCompute, Capability, ChangesetDraft, Client, ClientConfig, DocId, DraftBuilder,
    GeneratorSet, HttpConfig, HttpWps, LogEntry, NewScope, Op, ProblemKind, Prologue, Reference, ScopeEvent, ScopedOp,
    UnitSnapshot, WorldStore, WpsError,
};

/// A world-proto-server whose answers can be lost after sending, a replay's included, or whose sending can be lost
/// before it lands, and which can leave words out of its `features`, as a server without them would list them.
struct Lossy {
    inner: HttpWps,
    lose_answer: AtomicU32,
    lose_send: AtomicU32,
    hidden: Vec<&'static str>,
}

impl Lossy {
    fn take(counter: &AtomicU32) -> bool {
        counter.try_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1)).is_ok()
    }
}

impl Wps for Lossy {
    async fn healthz(&self) -> Result<String, WpsError> {
        self.inner.healthz().await
    }
    async fn protocol(&self) -> Result<WireProtocol, WpsError> {
        let mut protocol = self.inner.protocol().await?;
        if let Some(features) = &mut protocol.features {
            features.retain(|word| !self.hidden.contains(&word.as_str()));
        }
        Ok(protocol)
    }
    async fn protocol_defs(&self) -> Result<Value, WpsError> {
        self.inner.protocol_defs().await
    }
    async fn scopes(&self) -> Result<Vec<WireScope>, WpsError> {
        self.inner.scopes().await
    }
    async fn scopes_under(&self, under: &ScopePath) -> Result<Vec<WireScope>, WpsError> {
        self.inner.scopes_under(under).await
    }
    async fn create_scope(&self, path: &ScopePath, boundary: bool) -> Result<WireScope, WpsError> {
        self.inner.create_scope(path, boundary).await
    }
    async fn export_scope(&self, scope: &ScopePath, at: Option<Seq>) -> Result<RawExport, WpsError> {
        self.inner.export_scope(scope, at).await
    }
    async fn export_flagged(
        &self,
        scope: &ScopePath,
        flags: ExportFlags,
        at: &BTreeMap<ScopePath, Seq>,
    ) -> Result<Bytes, WpsError> {
        self.inner.export_flagged(scope, flags, at).await
    }
    async fn entity(&self, scope: &ScopePath, id: &EntityId, at: Option<Seq>) -> Result<WireEntity, WpsError> {
        self.inner.entity(scope, id, at).await
    }
    async fn trail(&self, scope: &ScopePath, id: &EntityId) -> Result<Option<WireTrail>, WpsError> {
        self.inner.trail(scope, id).await
    }
    async fn entities(
        &self,
        scope: &ScopePath,
        query: &EntityQuery,
    ) -> Result<(Vec<WireListed>, Option<String>), WpsError> {
        self.inner.entities(scope, query).await
    }
    async fn referrers(
        &self,
        scope: &ScopePath,
        id: &EntityId,
        at: Option<Seq>,
        after: Option<String>,
    ) -> Result<(Vec<WireReferrer>, Option<String>), WpsError> {
        self.inner.referrers(scope, id, at, after).await
    }
    async fn resolve(&self, scope: &ScopePath, reference: &str) -> Result<Resolution, WpsError> {
        self.inner.resolve(scope, reference).await
    }
    async fn changesets(&self, scope: &ScopePath, before: Option<Seq>, limit: u16) -> Result<Vec<LogEntry>, WpsError> {
        self.inner.changesets(scope, before, limit).await
    }
    async fn changeset_named(&self, scope: &ScopePath, operation: &str) -> Result<Option<LogEntry>, WpsError> {
        self.inner.changeset_named(scope, operation).await
    }
    async fn post_changeset(&self, changeset: &WireChangeset) -> Result<WireApplied, PostFailure> {
        if Self::take(&self.lose_send) {
            return Err(PostFailure::Lost { detail: "the test cut the connection before sending".to_owned() });
        }
        let answer = self.inner.post_changeset(changeset).await;
        if answer.is_ok() && Self::take(&self.lose_answer) {
            return Err(PostFailure::Lost { detail: "the test cut the connection after the send".to_owned() });
        }
        answer
    }
    async fn import(&self, scope: &ScopePath, base: Seq, document: &Value) -> Result<WireImport, WpsError> {
        self.inner.import(scope, base, document).await
    }
    async fn components(&self) -> Result<Vec<WireDefinition>, WpsError> {
        self.inner.components().await
    }
    async fn component(&self, name: &str) -> Result<Option<WireComponent>, WpsError> {
        self.inner.component(name).await
    }
    async fn generators(&self) -> Result<WireGenerators, WpsError> {
        self.inner.generators().await
    }
    async fn generator(&self, name: &str) -> Result<WireGenerator, WpsError> {
        self.inner.generator(name).await
    }
    async fn subscribe(&self, scope: &ScopePath) -> Result<EventStream, WpsError> {
        self.inner.subscribe(scope).await
    }
}

fn server(var: &str) -> Option<HttpWps> {
    let Ok(url) = env::var(var) else {
        eprintln!("skipped: {var} is unset, so there is no world-proto-server to hold this statement to");
        return None;
    };
    let routes = Routes::new(Url::parse(&url).expect("a URL"), RouteShape::Path).expect("routes");
    Some(HttpWps::new(HttpConfig::new(routes)).expect("an HTTP client"))
}

fn config(revision: Option<&str>) -> ClientConfig {
    ClientConfig {
        revision: revision.map(str::to_owned).or_else(|| env::var("STUDIO_WPS_REVISION").ok()),
        reconnect_first: Duration::from_millis(200),
        reconnect_last: Duration::from_secs(2),
        ..ClientConfig::default()
    }
}

async fn live(var: &str, config: ClientConfig) -> Option<Client<Lossy>> {
    live_hiding(var, config, Vec::new()).await
}

async fn live_hiding(var: &str, config: ClientConfig, hidden: Vec<&'static str>) -> Option<Client<Lossy>> {
    let inner = server(var)?;
    let lossy = Lossy { inner, lose_answer: AtomicU32::new(0), lose_send: AtomicU32::new(0), hidden };
    Some(
        Client::start(lossy, config, Arc::new(BlockingCompute))
            .await
            .expect("the probe passes and the schema cache loads"),
    )
}

/// A slug no earlier run used.
fn unique(prefix: &str) -> String {
    format!("{prefix}{}", &DeclaredId::mint().to_string()[..8])
}

/// An operation identifier no earlier run used, since WPS never frees one.
fn operation(name: &str) -> String {
    unique(&format!("{name}-"))
}

fn main() -> Pin {
    Pin::Branch(BranchName::main())
}

async fn read_at_heads<W: WorldStore>(store: &W, root: &ScopePath) -> UnitSnapshot {
    let cut = store.resolve_cut(root, &main()).await.expect("a cut");
    store.read_unit(&cut).await.expect("the site's scopes")
}

fn sub(root: &ScopePath, below: &str) -> ScopePath {
    path(&format!("{root}/{below}"))
}

async fn heads<W: WorldStore>(store: &W, root: &ScopePath) -> BTreeMap<ScopePath, Seq> {
    store.list_scopes(root).await.expect("a listing").into_iter().map(|scope| (scope.path, scope.head)).collect()
}

/// The reference locations of a registered schema: every field whose schema is one of the protocol's reference
/// values, as a pointer with `/*` for each element and `/{key}` for each value of a map.
fn reference_locations(schema: &Value) -> Vec<String> {
    const REFERENCES: [&str; 11] = [
        "values_entity_ref",
        "values_entity_id",
        "values_cell_address",
        "values_declared_entity_id",
        "values_derived_entity_id",
        "values_derivation_address",
        "values_local_derivation_address",
        "values_local_ref",
        "entity_id",
        "derived_entity_id",
        "reference",
    ];
    fn walk(node: &Value, at: &str, defs: &Map<String, Value>, stack: &mut Vec<String>, out: &mut Vec<String>) {
        let Some(fields) = node.as_object() else { return };
        if let Some(target) = fields.get("$ref").and_then(Value::as_str) {
            let name = target.rsplit('/').next().unwrap_or_default().to_owned();
            if target.starts_with("#/$defs/") && REFERENCES.contains(&name.as_str()) {
                out.push(at.to_owned());
                return;
            }
            if let Some(def) = defs.get(&name).filter(|_| !stack.contains(&name)) {
                stack.push(name);
                walk(def, at, defs, stack, out);
                stack.pop();
            }
        }
        for key in ["anyOf", "oneOf", "allOf"] {
            for sub in fields.get(key).and_then(Value::as_array).into_iter().flatten() {
                walk(sub, at, defs, stack, out);
            }
        }
        for (name, sub) in fields.get("properties").and_then(Value::as_object).into_iter().flatten() {
            walk(sub, &format!("{at}/{name}"), defs, stack, out);
        }
        if let Some(items) = fields.get("items").filter(|items| items.is_object()) {
            walk(items, &format!("{at}/*"), defs, stack, out);
        }
        if let Some(values) = fields.get("additionalProperties").filter(|values| values.is_object()) {
            walk(values, &format!("{at}/{{key}}"), defs, stack, out);
        }
    }
    let defs = schema.get("$defs").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut out = Vec::new();
    walk(schema, "", &defs, &mut Vec::new(), &mut out);
    out
}

/// Statement 10: every route this client uses answers in a shape it decodes, and every registered component's
/// reference fields are covered by its annotations or the walker's own list.
#[tokio::test]
async fn the_contract_suite() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let capabilities = client.capabilities().await.expect("capabilities");
    assert_eq!(capabilities.proto_version, "0.7.0");
    assert!(capabilities.has(Capability::ScopePaths) && capabilities.has(Capability::Generation), "{capabilities:?}");
    let wps = client.wps();
    assert!(wps.protocol_defs().await.expect("the definitions").get("$defs").is_some());
    let generators = wps.generators().await.expect("the generator family");
    assert!(!generators.generators.is_empty(), "the demo seed publishes generators");
    assert!(!client.registry().snapshot().generators.0.is_empty(), "the schema cache holds the generator set");
    for name in wps_client::schema::PRELOAD {
        let entry =
            wps.component(name).await.expect("a registry read").unwrap_or_else(|| panic!("{name} is registered"));
        assert!(entry.versions.iter().any(|version| version.version == "0.1.0" || *name == "external_ids"), "{name}");
    }

    let mut uncovered = Vec::new();
    for definition in wps.components().await.expect("the registry") {
        let component = wps.component(&definition.component).await.expect("a component").expect("listed, so defined");
        let Some(latest) = component.versions.iter().max_by_key(|version| SemVer::parse(&version.version).ok()) else {
            continue;
        };
        for location in reference_locations(&latest.schema) {
            let annotated = latest.refs.iter().any(|annotation| annotation.path == location);
            let listed = OWN_LIST.iter().any(|(owner, at)| *owner == definition.component && location.starts_with(at));
            if !annotated && !listed {
                uncovered.push(format!("{}{location}", definition.component));
            }
        }
    }
    assert!(uncovered.is_empty(), "reference fields neither annotated nor on the walker's list: {uncovered:?}");

    let dallas = path("walmart/dallas");
    // Scopes phase 3's subtree listing: the scopes at or below one path, and nothing for a path no scope has.
    let under = wps.scopes_under(&path("walmart")).await.expect("the subtree listing");
    let listed: Vec<&str> = under.iter().map(|row| row.scope.as_str()).collect();
    assert_eq!(listed, vec!["walmart", "walmart/dallas"]);
    assert!(wps.scopes_under(&path(&unique("absent/x"))).await.expect("a listing").is_empty());
    let cut = client.resolve_cut(&dallas, &main()).await.expect("the Dallas seed's cut");
    assert_eq!(client.resolve_pin(&dallas, &main()).await.expect("a pin"), cut.seqs[&dallas]);
    let snapshot = client.read_unit(&cut).await.expect("the Dallas seed read whole");
    let site = &snapshot.scopes[&dallas];
    assert!(site.prologue.is_some() && !site.entities.is_empty() && !site.records.is_empty());
    let entity = site.entities[0].id;
    let reading = client.entity(&dallas, &entity.into(), cut.seqs[&dallas]).await.expect("an entity read");
    assert_eq!(reading.id, EntityId::from(entity));
    let derived = client.derived(&dallas, cut.seqs[&dallas], None, None).await.expect("the derived listing");
    assert!(!derived.items.is_empty(), "the Dallas seed's generation derives entities");
    let target = site.entities.iter().find(|entity| entity.components.contains_key("frame")).expect("a frame").id;
    client.referrers(&dallas, &target.into(), cut.seqs[&dallas], None).await.expect("a referrer page");
    assert_eq!(client.history(&dallas, None, 1).await.expect("the log")[0].seq, cut.seqs[&dallas]);
    let export = client.export(&cut).await.expect("the Dallas seed's export is closed");
    assert!(export.scopes.contains_key(&path("module/go-catalog")), "the stamps' pinned catalog, at their pin");
    let mut events = client.events(&dallas);
    assert!(timeout(Duration::from_millis(300), events.next()).await.is_err(), "the stream opens and stays quiet");
    let pinned_within = format!("{entity}@{}", cut.seqs[&dallas]);
    let refusal = wps.resolve(&dallas, &pinned_within).await.expect_err("a pin within its unit is the rule's refusal");
    let owned = unresolved(&refusal).map(|(unresolved, _)| unresolved);
    assert_eq!(owned, Some(Unresolved::Owned { owner: dallas.clone(), problem: None }), "{refusal:?}");
    let unnamed = wps.changeset_named(&dallas, "no-such-operation").await.expect("the log read by operation");
    assert!(unnamed.is_none());

    let scope = path(&format!("template/site/{}", unique("contract")));
    for prefix in ["template", "template/site"] {
        client.ensure_scope(&path(prefix), false).await.expect("a prefix");
    }
    client.ensure_scope(&scope, true).await.expect("the template's root");
    let document = json!({ "proto_version": "0.7.0", "doc": { "id": DocId::mint().as_str(), "kind": "world" }, "entities": [
        { "id": DeclaredId::mint().to_string(), "components": { "template.description": { "title": "Contract", "text": "a test" } } }
    ] });
    let proposal = client.propose_import(&scope, Seq::new(0), &document).await.expect("an import proposal");
    assert_eq!(proposal.tally.declared, 1);
}

async fn create_draft(base: &UnitSnapshot, root: &ScopePath, operation: &str) -> ChangesetDraft {
    let mut builder = DraftBuilder::new(
        base,
        root.clone(),
        Author::new("integration").expect("a word"),
        "Lay out a frame",
        operation,
    );
    builder.declare(
        &sub(root, "map"),
        Some("frame".to_owned()),
        object(json!({ "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } })),
    );
    builder.build()
}

/// A site's scopes under a customer no earlier run used, empty at changeset 0, made by name as a fixture.
async fn fresh_site(client: &Client<Lossy>, name: &str) -> ScopePath {
    let root = path(&format!("customer/{}/site/{name}", unique("t")));
    ensure_prefixes(client, &root).await.expect("the prefixes");
    client.ensure_scope(&root, true).await.expect("the root");
    for below in SUB_SCOPES {
        client.ensure_scope(&sub(&root, below), false).await.expect("a sub-scope");
    }
    root
}

/// Statement 11, the operation identifier: a resend is answered 201 with the first result and `replayed`, after the
/// head moved too and whatever the body's key order, whitespace and number spelling; another body under the same
/// operation is `OperationReused`, naming the changeset the identifier holds; an answer lost once settles by the
/// resend; both answers lost leave the outcome unknown, and a client that never saw an answer, as after a restart,
/// sends the kept body again and is answered the replay.
#[tokio::test]
async fn the_operation_identifier_replays() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let capabilities = client.capabilities().await.expect("capabilities");
    assert!(capabilities.has(Capability::OperationId), "master 60aa424 and later list operation_id: {capabilities:?}");
    let root = fresh_site(&client, "replay").await;
    let map = sub(&root, "map");
    let base = read_at_heads(&client, &root).await;
    let draft = create_draft(&base, &root, &operation("replay-1")).await;
    let changeset = changeset_of(&draft).expect("a body");
    let first = client.wps().post_changeset(&changeset).await.expect("the first sending");
    let replay = client.wps().post_changeset(&changeset).await.expect("a replay is answered");
    assert!(replay.replayed && replay.seq == first.seq, "{replay:?}");
    let other = create_draft(&read_at_heads(&client, &root).await, &root, &operation("replay-2")).await;
    client.apply(&other).await.expect("the head moves");
    let late = client.wps().post_changeset(&changeset).await.expect("a replay after the head moved");
    assert!(late.replayed && late.seq == first.seq, "{late:?}");
    let mut changed = changeset.clone();
    changed.message = "another body".to_owned();
    let reused = client.wps().post_changeset(&changed).await;
    let named = first.scopes.iter().map(|part| (part.scope.clone(), part.seq)).collect();
    let held = changeset.operation.clone().expect("every write names one");
    let expected = WpsError::OperationReused { operation: held, initiated: changeset.scope.clone(), named };
    assert_eq!(reused, Err(PostFailure::Answered(expected)));

    // The same body respelled: keys in another order, other whitespace, `3` written `3.0`.
    let mut body: Value = serde_json::to_value(&changeset).expect("a body");
    let frame = &mut body["ops"][0]["components"]["frame"];
    frame["dims"] = json!(3.0);
    let mut respelled = String::from("{ ");
    let fields: Vec<String> = body
        .as_object()
        .expect("an object")
        .iter()
        .rev()
        .map(|(key, value)| format!("\n  {}: {}", json!(key), serde_json::to_string_pretty(value).expect("JSON")))
        .collect();
    respelled.push_str(&fields.join(" ,"));
    respelled.push_str(" }");
    // The route names the scope the changeset is initiated on, as its body does: the site's root.
    let url = client.wps().inner.routes().scope(&changeset.scope, "changesets");
    let answer = RawHttp::new()
        .post(url)
        .header("content-type", "application/json")
        .body(respelled)
        .send()
        .await
        .expect("an answer");
    assert_eq!(answer.status().as_u16(), 201);
    let canonical: Value = serde_json::from_slice(&answer.bytes().await.expect("a body")).expect("JSON");
    assert_eq!((canonical["replayed"].clone(), canonical["seq"].clone()), (json!(true), json!(first.seq)));

    client.wps().lose_answer.store(1, Ordering::SeqCst);
    let lost = create_draft(&read_at_heads(&client, &root).await, &root, &operation("replay-3")).await;
    let applied = client.apply(&lost).await.expect("a lost answer settles by the resend");
    assert!(applied.replayed);

    client.wps().lose_answer.store(2, Ordering::SeqCst);
    let twice = create_draft(&read_at_heads(&client, &root).await, &root, &operation("replay-4")).await;
    let kept = changeset_of(&twice).expect("the body the approval keeps");
    let unknown = client.apply(&twice).await.expect_err("both answers lost");
    assert_eq!(unknown, WpsError::OutcomeUnknown { scopes: vec![map.clone()] });
    let moving = create_draft(&read_at_heads(&client, &root).await, &root, &operation("replay-5")).await;
    client.apply(&moving).await.expect("the head moves");

    let restarted = live("STUDIO_TEST_WPS_URL", config(None)).await.expect("a second client of the same server");
    let again = restarted.resend(&kept, &twice).await.expect("the replay, after the head moved");
    assert_eq!((again.seqs[&map], again.replayed), (Seq::new(4), true), "{again:?}");
    assert_eq!(heads(&client, &root).await[&map], Seq::new(5), "every draft landed once");
}

/// Section 3.4's exact replace: `overwrite` is a `Replace`, and leaves the entity carrying exactly what it states, a
/// label it omits cleared; a replace of an id not live in its scope is refused as `ReplaceMissingTarget`.
#[tokio::test]
async fn exact_replace_leaves_exactly_what_it_states() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let root = fresh_site(&client, "replace").await;
    let map = sub(&root, "map");
    let base = read_at_heads(&client, &root).await;
    let mut builder = DraftBuilder::new(
        &base,
        root.clone(),
        Author::new("integration").expect("a word"),
        "A labelled frame",
        operation("replace-1"),
    );
    let frame = builder.declare(
        &map,
        Some("labelled".to_owned()),
        object(json!({ "frame": { "dims": 3, "units": "mm", "unit_mm": 1 }, "external_ids": { "src": "f" } })),
    );
    client.apply(&builder.build()).await.expect("declared");
    let base = read_at_heads(&client, &root).await;
    let mut builder = DraftBuilder::new(
        &base,
        root.clone(),
        Author::new("integration").expect("a word"),
        "Overwrite the frame",
        operation("replace-2"),
    );
    builder
        .overwrite(frame, None, object(json!({ "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } })))
        .expect("kept");
    let overwrite = builder.build();
    assert!(matches!(overwrite.ops[0].op, Op::Replace { .. }));
    assert_eq!(client.check(&overwrite, &base).await.expect("check"), Vec::new());
    client.apply(&overwrite).await.expect("replaced");
    let held = client.entity(&map, &frame.into(), Seq::new(2)).await.expect("the frame");
    assert_eq!(held.components.keys().cloned().collect::<Vec<_>>(), vec!["frame".to_owned()]);
    assert_eq!(held.name, None, "the omitted label is cleared");

    let stray = WireChangeset::of_one_scope(
        map.clone(),
        Seq::new(2),
        "integration",
        "replace nothing",
        vec![Op::Replace { id: DeclaredId::mint(), name: None, components: Map::new() }],
    );
    let refused = client.wps().post_changeset(&stray).await;
    assert!(
        matches!(&refused, Err(PostFailure::Answered(WpsError::Refused { problem })) if problem.kind == ProblemKind::ReplaceMissingTarget),
        "{refused:?}"
    );
}

/// Statement 12: against master, `preview` of a draft whose references stay inside the site sends no request.
#[tokio::test]
async fn preview_sends_no_request() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let root = fresh_site(&client, "preview").await;
    let base = read_at_heads(&client, &root).await;
    let mut builder = DraftBuilder::new(
        &base,
        root.clone(),
        Author::new("integration").expect("a word"),
        "Card",
        operation("preview-1"),
    );
    let node = builder.declare(&sub(&root, "flow"), None, object(json!({ "flow.return_rule": { "min_backlog": 2 } })));
    let frame = builder.declare(
        &sub(&root, "map"),
        None,
        object(json!({ "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } })),
    );
    builder.declare(
        &root,
        None,
        object(json!({ "template.description": { "title": "Preview", "text": node.to_string() } })),
    );
    builder.prologue(
        &root,
        Prologue {
            doc: DocId::mint(),
            kind: Some("world".to_owned()),
            name: Some("Preview".to_owned()),
            primary_frame: Some(frame.into()),
            depends_on: Vec::new(),
        },
    );
    let draft = builder.build();
    // The schema cache loads `frame` on first use; that registry read is the cache's, never a preview's.
    client.registry().ensure(client.wps(), "frame").await.expect("the frame's schema");
    let before = client.wps().inner.requests_sent();
    let preview = client.preview(&draft, &base).await.expect("a preview");
    assert_eq!(client.wps().inner.requests_sent(), before, "no request reached world-proto-server");
    assert_eq!(serde_json::to_value(preview.checked_by).expect("a word"), json!("agent-studio"));
    assert!(preview.problems.is_empty(), "{:?}", preview.problems);
}

/// Statement 13: a listener paused while 300 changesets apply yields a `Moved` with `gap` set, or `Resynced`.
#[tokio::test]
async fn a_paused_listener_hears_of_its_gap() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", ClientConfig { event_backlog: 16, ..config(None) }).await else {
        return;
    };
    let root = fresh_site(&client, "paused").await;
    let map = sub(&root, "map");
    let frame = DeclaredId::mint();
    let declare = WireChangeset::of_one_scope(
        map.clone(),
        Seq::new(0),
        "t",
        "a frame",
        vec![Op::Declare {
            id: frame,
            name: None,
            components: object(json!({ "frame": { "dims": 2, "units": "mm" } })),
        }],
    );
    client.wps().post_changeset(&declare).await.expect("a frame");
    let mut paused = client.events(&map);
    tokio::time::sleep(Duration::from_millis(500)).await;
    // A consumer starts from the head it reads when first polled: polled once, it is live, then paused.
    assert!(timeout(Duration::from_millis(50), paused.next()).await.is_err(), "nothing moved yet");
    for seq in 1..=300u64 {
        let patch = WireChangeset::of_one_scope(
            map.clone(),
            Seq::new(seq),
            "t",
            format!("move {seq}"),
            vec![Op::Patch {
                id: frame,
                name: None,
                components: object(json!({ "frame": { "dims": 2, "units": "mm", "$comment": seq.to_string() } })),
            }],
        );
        client.wps().post_changeset(&patch).await.expect("a move");
    }
    let mut told = false;
    loop {
        let event = timeout(Duration::from_secs(5), paused.next()).await.expect("never silent").expect("an event");
        match event {
            ScopeEvent::Moved { seq, gap, .. } => {
                told |= gap;
                if seq == Seq::new(301) {
                    break;
                }
            }
            ScopeEvent::Resynced { head, .. } => {
                told = true;
                if head == Seq::new(301) {
                    break;
                }
            }
        }
    }
    assert!(told, "a listener 300 behind, holding 16, is told of the gap");
}

/// Spec 01 statement 2's second half, as the client serves it: a watcher hears `Moved` within 2 s of wall clock
/// after a changeset.
#[tokio::test]
async fn a_watcher_hears_a_changeset_within_two_seconds() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let root = fresh_site(&client, "watched").await;
    let mut events = client.events_since(&sub(&root, "map"), Seq::new(0));
    let draft = create_draft(&read_at_heads(&client, &root).await, &root, &operation("watched-1")).await;
    let sent = Instant::now();
    client.apply(&draft).await.expect("applied");
    let event = timeout(Duration::from_secs(2), events.next()).await.expect("within 2 s").expect("an event");
    let heard = match event {
        ScopeEvent::Moved { seq, .. } => seq,
        ScopeEvent::Resynced { head, .. } => head,
    };
    assert_eq!(heard, Seq::new(1));
    eprintln!("heard after {} ms", sent.elapsed().as_millis());
}

/// Section 3.6 at scopes phase 3: one changeset writing a site's four scopes moves each scope's feed once, every event
/// carrying the one commit place the changeset took; a later changeset carries a later one.
#[tokio::test]
async fn one_changeset_moves_each_scope_under_one_commit() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let root = fresh_site(&client, "commit").await;
    let scopes = [root.clone(), sub(&root, "map"), sub(&root, "flow"), sub(&root, "fleet")];
    let mut watches: Vec<_> = scopes.iter().map(|scope| client.events_since(scope, Seq::new(0))).collect();
    tokio::time::sleep(Duration::from_secs(1)).await;
    // Each consumer starts from the head it reads when first polled, so each is polled, and is quiet, before the write.
    for watch in &mut watches {
        assert!(timeout(Duration::from_millis(50), watch.next()).await.is_err(), "nothing moved yet");
    }
    let base = read_at_heads(&client, &root).await;
    let author = Author::new("integration").expect("a word");
    let mut builder = DraftBuilder::new(&base, root.clone(), author, "All four", operation("commit"));
    for scope in &scopes {
        builder.declare(scope, None, object(json!({ "external_ids": { "src": scope.to_string() } })));
    }
    client.apply(&builder.build()).await.expect("one changeset");
    let mut commits = Vec::new();
    for watch in &mut watches {
        let heard = timeout(Duration::from_secs(2), watch.next()).await.expect("within 2 s").expect("an event");
        let ScopeEvent::Moved { seq, commit, .. } = heard else { panic!("a move, not a resync: {heard:?}") };
        assert_eq!(seq, Seq::new(1));
        commits.push(commit.expect("world-proto-server's commit place"));
    }
    assert!(commits.windows(2).all(|pair| pair[0] == pair[1]), "one changeset, one commit place: {commits:?}");
    let map = WireChangeset::of_one_scope(
        sub(&root, "map"),
        Seq::new(1),
        "integration",
        "a later move",
        vec![Op::Declare {
            id: DeclaredId::mint(),
            name: None,
            components: object(json!({ "frame": { "dims": 2, "units": "mm" } })),
        }],
    );
    client.wps().post_changeset(&map).await.expect("the map moves");
    let heard = timeout(Duration::from_secs(2), watches[1].next()).await.expect("within 2 s").expect("an event");
    let ScopeEvent::Moved { commit, .. } = heard else { panic!("{heard:?}") };
    assert!(commit.expect("a commit place") > commits[0], "a later changeset takes a later place");
}

/// Create site as the API runs it: the body built over one read and kept, then sent; a prefix another test made after
/// the read (the tests share one server) refuses it, and a body built again over a new read is kept and sent instead.
async fn made(client: &Client<Lossy>, plan: &SitePlan) -> Result<(Applied, WireChangeset), WpsError> {
    let sent = site_changeset(client, plan).await?;
    match create_site(client, plan, &sent).await {
        Err(NotLanded::Refused(WpsError::ScopeExists { scope, .. })) if scope != plan.root => {
            let again = site_changeset(client, plan).await?;
            Ok((create_site(client, plan, &again).await.map_err(NotLanded::into_error)?, again))
        }
        answered => Ok((answered.map_err(NotLanded::into_error)?, sent)),
    }
}

/// The plan of a site's four scopes: in the map a 3-D frame, the primary one, a 2-D floor in it and a lane tiling the
/// floor, which the registered `tiling` generator derives cells from, and a sector of the lane; a flow node naming the
/// sector at head; a fleet entity; and `card` in the root, the root's prologue named "Fixture". It states `generators`.
fn site_plan(root: &ScopePath, generators: &GeneratorSet, card: Value, operation: &str) -> SitePlan {
    let (frame, floor, lane, sector) = (DeclaredId::mint(), DeclaredId::mint(), DeclaredId::mint(), DeclaredId::mint());
    let prologue = Prologue {
        doc: DocId::mint(),
        kind: Some("world".to_owned()),
        name: Some("Fixture".to_owned()),
        primary_frame: Some(frame.into()),
        depends_on: Vec::new(),
    };
    let author = Author::new("integration").expect("a word");
    let mut plan = SitePlan::new(root.clone(), prologue, author, "Create site", operation).expect("a site's root");
    let map = sub(root, "map");
    let entities = [
        (frame, "site", json!({ "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } })),
        (
            floor,
            "floor",
            json!({ "frame": {
                "dims": 2, "units": "mm", "parent": frame.to_string(),
                "transform_to_parent": { "scale": [1, 1], "translation": [0, 0, 0] }
            } }),
        ),
        (
            lane,
            "lane",
            json!({
                "region": { "frame": floor.to_string(), "origin": [0, 0], "size": [1800, 900] },
                "tiling": { "derives": { "cell": { "traversable_by": ["htm"] } }, "pitch": [900, 900] }
            }),
        ),
        (sector, "sector", json!({ "sector": { "members": [lane.to_string()] } })),
    ];
    for (id, name, components) in entities {
        plan.declare(&map, id, Some(name.to_owned()), object(components)).expect("the map");
    }
    let fleet = json!({ "external_ids": { "fixture": "fleet" } });
    plan.declare(&sub(root, "fleet"), DeclaredId::mint(), None, object(fleet)).expect("the fleet");
    let kin = Reference::Kin(sector.into()).spelled();
    let node = json!({ "flow.node": { "kind": "storage", "holds": "inventory_tote", "sectors": [kin] } });
    plan.declare(&sub(root, "flow"), DeclaredId::mint(), None, object(node)).expect("the flow");
    plan.declare(root, DeclaredId::mint(), None, object(card)).expect("the root");
    plan.expect_generators(generators.clone());
    plan
}

/// Statement 14 at scopes phase 3: Create site is ONE changeset that creates the customer's prefixes flagless, the
/// root a boundary and its sub-scopes flagless, and writes all four with their prologues from base 0, refused whole
/// (by a schema, or by a generator set other than the one that runs, leaving no scope behind) or applied whole, each
/// scope at changeset 1 after an answer cut once; `GET /scopes?under=` lists the four; the same body sent again is
/// answered with the whole first answer, and Create site called again from a restarted client with the plan and body
/// it kept answers the site it made, writing nothing; a sibling named is refused 422; the site lists under its name; a flow
/// draft whose map moved is refused `head-moved` by WPS inside the write; and the export loads in foundry's reader.
#[tokio::test]
async fn create_site_is_one_changeset_and_loads() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let customer = path(&format!("customer/{}", unique("t")));
    let root = sub(&customer, "site/fixture");
    let generators = client.registry().snapshot().generators.clone();
    let card = json!({ "template.description": { "title": "Fixture", "text": "a made site" } });
    let plan = site_plan(&root, &generators, card.clone(), &operation("create"));

    let refused = site_plan(&root, &generators, json!({ "template.description": { "title": 7 } }), &operation("bad"));
    let failure = made(&client, &refused).await.expect_err("the root's card is refused by its schema");
    assert!(
        matches!(&failure, WpsError::Refused { problem } if problem.kind == ProblemKind::SchemaInvalid),
        "{failure:?}"
    );
    let mut stale = generators.clone();
    let tiling = stale.0.get_mut("tiling").expect("the demo seed's tiling generator");
    tiling.0 = SemVer::new(0, 0, 1);
    let mismatched = site_plan(&root, &stale, card, &operation("stale"));
    let failure = made(&client, &mismatched).await.expect_err("another generator set runs");
    assert_eq!(failure, WpsError::GeneratorsMoved { expected: stale, found: generators.clone() });
    assert!(client.list_scopes(&customer).await.expect("a listing").is_empty(), "nothing created, no prefix either");

    let before = client.subtree(&path("customer")).await.expect("a read");
    let draft = plan.draft(&before).expect("the one changeset");
    assert_eq!(client.check(&draft, &plan.empty_base()).await.expect("check"), Vec::new());
    client.wps().lose_answer.store(1, Ordering::SeqCst);
    let (applied, sent) = made(&client, &plan).await.expect("one changeset, its answer lost once");
    let expected: BTreeMap<ScopePath, Seq> = plan.scopes().into_iter().map(|scope| (scope, Seq::new(1))).collect();
    assert_eq!((applied.seqs.clone(), applied.replayed), (expected, true), "written once, the resend replayed");
    let made: Vec<(String, bool)> =
        applied.created.iter().map(|scope| (scope.scope.to_string(), scope.boundary)).collect();
    for (scope, boundary) in [(customer.clone(), false), (sub(&customer, "site"), false), (root.clone(), true)] {
        assert!(made.contains(&(scope.to_string(), boundary)), "{scope} created with {boundary}: {made:?}");
    }
    let ran = applied.generation[&sub(&root, "map")].as_ref().expect("the map's generation ran");
    assert_eq!(ran.generators, generators, "it ran the stated set");

    let url = client.wps().inner.routes().scopes_under(&root);
    let answer = RawHttp::new().get(url).send().await.expect("an answer").bytes().await.expect("a body");
    let listed: Value = serde_json::from_slice(&answer).expect("JSON");
    let rows: Vec<(String, bool, String, u64)> = listed["scopes"]
        .as_array()
        .expect("the subtree")
        .iter()
        .map(|row| {
            let text = |key: &str| row[key].as_str().expect("a string").to_owned();
            (
                text("scope"),
                row["boundary"].as_bool().expect("a flag"),
                text("unit"),
                row["head"].as_u64().expect("a head"),
            )
        })
        .collect();
    let four: Vec<(String, bool, String, u64)> = [("", true), ("/fleet", false), ("/flow", false), ("/map", false)]
        .map(|(below, boundary)| (format!("{root}{below}"), boundary, root.to_string(), 1))
        .into();
    assert_eq!(rows, four, "GET /scopes?under= lists the four, each at changeset 1 in the root's unit");

    let replay = client.wps().post_changeset(&sent).await.expect("the same body again");
    let parts: Vec<(ScopePath, Seq)> = replay.scopes.iter().map(|part| (part.scope.clone(), part.seq)).collect();
    assert_eq!(parts, plan.scopes().into_iter().map(|scope| (scope, Seq::new(1))).collect::<Vec<_>>());
    assert_eq!((replay.replayed, replay.created), (true, applied.created.clone()), "the whole first answer");
    let restarted = live("STUDIO_TEST_WPS_URL", config(None)).await.expect("a second client, as after a restart");
    let kept: SitePlan = serde_json::from_str(&serde_json::to_string(&plan).expect("JSON")).expect("the kept plan");
    let body: WireChangeset = serde_json::from_str(&serde_json::to_string(&sent).expect("JSON")).expect("the body");
    let again = create_site(&restarted, &kept, &body).await.expect("the site the first call made");
    assert_eq!((again.seqs, again.replayed), (applied.seqs.clone(), true), "answered again, nothing written");
    assert!(heads(&client, &root).await.values().all(|head| *head == Seq::new(1)));

    let flow = sub(&root, "flow");
    let mut sibling = sent.clone();
    sibling.operation = Some(operation("sibling"));
    sibling.scope = sub(&root, "map");
    sibling.create.clear();
    sibling.bases.clear();
    sibling.prologues.clear();
    sibling.generators = None;
    sibling.ops.retain(|wire| wire.scope.as_ref() == Some(&flow));
    let refused = client.wps().post_changeset(&sibling).await;
    let named = matches!(&refused, Err(PostFailure::Answered(WpsError::Refused { problem }))
        if problem.kind == ProblemKind::OutsideTheInitiatingScope && problem.scope == Some(flow.clone()));
    assert!(named, "a sibling named is refused 422: {refused:?}");

    let customer_slug = customer.slugs().nth(1).expect("the customer's slug").to_owned();
    let sites = list_sites(&client, Some(&customer_slug)).await.expect("the site list");
    assert_eq!(
        sites.iter().map(|site| (site.root.clone(), site.name.clone())).collect::<Vec<_>>(),
        vec![(root.clone(), Some("Fixture".to_owned()))]
    );

    let base = read_at_heads(&client, &root).await;
    let author = Author::new("integration").expect("a word");
    let mut builder = DraftBuilder::new(&base, root.clone(), author, "A flow edit", operation("flow"));
    builder.declare(&flow, None, object(json!({ "flow.return_rule": { "min_backlog": 3 } })));
    let flow_edit = builder.build();
    let frame = json!({ "frame": { "dims": 2, "units": "mm" } });
    let other = WireChangeset::of_one_scope(
        sub(&root, "map"),
        Seq::new(1),
        "someone-else",
        "moved",
        vec![Op::Declare { id: DeclaredId::mint(), name: None, components: object(frame) }],
    );
    client.wps().post_changeset(&other).await.expect("the map moves");
    let refused = client.apply(&flow_edit).await.expect_err("the map it read moved");
    assert_eq!(refused, WpsError::HeadMoved { moved: vec![(sub(&root, "map"), Seq::new(1), Seq::new(2))] });
    assert_eq!(heads(&client, &root).await[&flow], Seq::new(1), "nothing written");

    let cut = client.resolve_cut(&root, &main()).await.expect("a cut");
    let export = client.export(&cut).await.expect("a closed export");
    let file: Value = serde_json::from_slice(&export.bytes).expect("the export is JSON");
    let world = flagged_export::world(Path::new("made-site"), &file).expect("foundry's reader builds the made site");
    assert!(world.frames.primary().is_some(), "and finds its primary frame, the one frame that states unit_mm");
}

/// A builder of one draft over `base`, the site at `root`'s, under an operation identifier named after `name`.
fn builder<'a>(base: &'a UnitSnapshot, root: &ScopePath, message: &str, name: &str) -> DraftBuilder<'a> {
    let author = Author::new("integration").expect("a word");
    DraftBuilder::new(base, root.clone(), author, message, operation(name))
}

/// A flow node standing for one sector, named at head.
fn node_naming(sector: &str) -> Map<String, Value> {
    object(json!({ "flow.node": { "kind": "storage", "holds": "inventory_tote", "sectors": [sector] } }))
}

/// Section 3.4, a gate the mock leaves to WPS (wp:server/DESIGN.md section 8, the dangling refusal): a deletion that
/// a flow node of the same unit still names at head is refused whole, naming the sector, and nothing is written.
#[tokio::test]
async fn a_deletion_still_named_at_head_is_refused() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let root = fresh_site(&client, "named").await;
    let (map, flow) = (sub(&root, "map"), sub(&root, "flow"));
    let base = read_at_heads(&client, &root).await;
    let mut first = builder(&base, &root, "A sector and its node", "named-1");
    let sector = first.declare(&map, None, object(json!({ "sector": { "members": [] } })));
    let kin = first.reference(&Reference::Kin(sector.into()));
    first.declare(&flow, None, node_naming(&kin));
    client.apply(&first.build()).await.expect("a node naming its sector at head");
    let base = read_at_heads(&client, &root).await;
    let mut drop = builder(&base, &root, "Drop the sector", "named-2");
    drop.delete(sector).expect("a live sector");
    let refused = client.apply(&drop.build()).await.expect_err("the node still names the sector");
    let left = matches!(&refused, WpsError::Refused { problem }
        if problem.kind == ProblemKind::LeftDangling && problem.entity == Some(sector.into()));
    assert!(left, "{refused:?}");
    assert_eq!(heads(&client, &root).await[&map], Seq::new(1), "nothing written");
}

/// Section 3.4, a gate the mock leaves to WPS (wp:server/DESIGN.md section 8, step 7): every reference a changeset
/// writes resolves against the state the changeset leaves, so a node naming at head a sector the same changeset
/// deletes is refused whole, naming the node.
#[tokio::test]
async fn a_reference_to_what_its_changeset_deletes_is_refused() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let root = fresh_site(&client, "final").await;
    let (map, flow) = (sub(&root, "map"), sub(&root, "flow"));
    let base = read_at_heads(&client, &root).await;
    let mut first = builder(&base, &root, "A sector", "final-1");
    let sector = first.declare(&map, None, object(json!({ "sector": { "members": [] } })));
    client.apply(&first.build()).await.expect("a sector");
    let base = read_at_heads(&client, &root).await;
    let mut both = builder(&base, &root, "Drop the sector and name it", "final-2");
    both.delete(sector).expect("a live sector");
    let kin = both.reference(&Reference::Kin(sector.into()));
    let node = both.declare(&flow, None, node_naming(&kin));
    let refused = client.apply(&both.build()).await.expect_err("the node names a sector the changeset deletes");
    let dangling = matches!(&refused, WpsError::Refused { problem }
        if problem.kind == ProblemKind::Dangling && problem.entity == Some(node.into()));
    assert!(dangling, "{refused:?}");
    let after = heads(&client, &root).await;
    assert_eq!((after[&map], after[&flow]), (Seq::new(1), Seq::new(0)), "nothing written");
}

/// Section 3.4, a gate the mock leaves to WPS (wp:server/DESIGN.md section 8, step 6): two sites made by one changeset,
/// each frame's parent the other's frame pinned at the other site's new changeset, pin one another in a cycle, refused
/// whole, so neither site is made.
#[tokio::test]
async fn sites_pinning_one_another_in_one_changeset_are_a_pin_cycle() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let sites = path(&format!("customer/{}/site", unique("t")));
    let (a, b) = (sub(&sites, "a"), sub(&sites, "b"));
    ensure_prefixes(&client, &a).await.expect("the prefixes");
    let (x, y) = (DeclaredId::mint(), DeclaredId::mint());
    let frame =
        |parent: DeclaredId| object(json!({ "frame": { "dims": 2, "units": "mm", "parent": format!("{parent}@1") } }));
    let cycle = ChangesetDraft {
        initiated_on: sites.clone(),
        bases: [&sites, &a, &b].map(|scope| (scope.clone(), Seq::new(0))).into(),
        creates: vec![NewScope { path: a.clone(), boundary: true }, NewScope { path: b.clone(), boundary: true }],
        prologues: BTreeMap::new(),
        author: Author::new("integration").expect("a word"),
        message: "Two sites pinning one another".to_owned(),
        ops: vec![
            ScopedOp { scope: a.clone(), op: Op::Declare { id: x, name: None, components: frame(y) } },
            ScopedOp { scope: b.clone(), op: Op::Declare { id: y, name: None, components: frame(x) } },
        ],
        expect_generators: None,
        operation: operation("cycle"),
        minted: vec![x, y],
    };
    let refused = client.apply(&cycle).await.expect_err("a pin cycle");
    assert!(matches!(&refused, WpsError::Refused { problem } if problem.kind == ProblemKind::PinCycle), "{refused:?}");
    let listed: Vec<ScopePath> =
        client.list_scopes(&sites).await.expect("a listing").into_iter().map(|s| s.path).collect();
    assert_eq!(listed, vec![sites.clone()], "neither site was made");
}

/// Section 3.4's read bases, against WPS (wp:server/DESIGN.md section 8, step 3): a changeset may state the head it
/// read of the generator family; a base behind the family's head is refused 409, naming the family with both
/// changesets, and nothing is written; at the head the changeset applies.
#[tokio::test]
async fn a_registry_family_read_base_is_held_to_its_head() {
    let Some(client) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let root = fresh_site(&client, "family").await;
    let map = sub(&root, "map");
    let family = path("registry/generators");
    let head = Seq::new(client.wps().generators().await.expect("the generator family").at);
    let behind = Seq::new(head.get().checked_sub(1).expect("the demo seed published generators"));
    let base = read_at_heads(&client, &root).await;
    let mut framed = builder(&base, &root, "A frame read against the generators", "family");
    framed.declare(&map, None, object(json!({ "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } })));
    let mut draft = framed.build();
    draft.bases.insert(family.clone(), behind);
    let refused = client.apply(&draft).await.expect_err("the family moved past the base");
    assert_eq!(refused, WpsError::HeadMoved { moved: vec![(family.clone(), behind, head)] });
    assert_eq!(heads(&client, &root).await[&map], Seq::new(0), "nothing written");
    draft.bases.insert(family, head);
    let applied = client.apply(&draft).await.expect("at the family's head, under the identifier nothing stored");
    assert_eq!(applied.seqs[&map], Seq::new(1));
}

/// Statement 18: the seed, on two fresh instances, writes the same entities, ids included; again, it skips every
/// entry; a component of ours missing from the registry, or an entry WPS refuses, stops it, naming it.
#[tokio::test]
async fn the_seed_writes_one_world_and_skips_after() {
    let Some(first) = live("STUDIO_TEST_WPS_URL", config(None)).await else { return };
    let folder = env::temp_dir().join(unique("studio-seed-"));
    fs::create_dir_all(&folder).expect("a folder");
    let scope = path(&format!("template/site/{}", unique("seed")));
    let below = sub(&scope, "map");
    let (doc, card) = (DocId::mint(), DeclaredId::mint());
    let document = json!({ "proto_version": "0.7.0", "doc": { "id": doc.as_str(), "kind": "world", "name": "Seed fixture" }, "entities": [
        { "id": card.to_string(), "components": { "template.description": { "title": "Seed", "text": "one entry" } } }
    ] });
    fs::write(folder.join("fixture.world.json"), document.to_string()).expect("the document");
    let map_document = json!({ "proto_version": "0.7.0", "doc": { "id": DocId::mint().as_str(), "kind": "world" }, "entities": [
        { "id": DeclaredId::mint().to_string(), "components": { "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } } }
    ] });
    fs::write(folder.join("map.world.json"), map_document.to_string()).expect("the map's document");
    // The second entry sits under the first one's scope, which by then holds entities.
    let manifest = |vocabulary: &[&str], document: &str| {
        json!({ "v": 1, "vocabulary": vocabulary, "entries": [
            { "scope": scope.to_string(), "document": document },
            { "scope": below.to_string(), "document": "map.world.json" }
        ] })
        .to_string()
    };
    fs::write(folder.join("manifest.json"), manifest(&["template.description"], "fixture.world.json"))
        .expect("the manifest");
    let vocabulary = env::var("WORLD_PROTO_DIR").ok().map(|dir| PathBuf::from(dir).join("vocabulary"));
    let report = seed::run(&first, &folder.join("manifest.json"), vocabulary.as_deref(), "test").await.expect("seeded");
    assert_eq!(report.written, vec![(scope.clone(), Seq::new(1)), (below.clone(), Seq::new(1))]);
    let again =
        seed::run(&first, &folder.join("manifest.json"), vocabulary.as_deref(), "test").await.expect("seeded again");
    assert_eq!((again.written.len(), again.skipped.clone()), (0, vec![scope.clone(), below.clone()]));

    match live("STUDIO_TEST_WPS_SECOND_URL", config(None)).await {
        Some(second) => {
            seed::run(&second, &folder.join("manifest.json"), vocabulary.as_deref(), "test")
                .await
                .expect("seeded twice");
            let (one, two) = (
                first.read(&scope, Seq::new(1)).await.expect("read"),
                second.read(&scope, Seq::new(1)).await.expect("read"),
            );
            assert_eq!(one.entities, two.entities, "the same entities, ids included");
            assert_eq!(one.prologue, two.prologue);
        }
        None => eprintln!("statement 18's two instances are unverified"),
    }

    fs::write(folder.join("missing.json"), manifest(&["template.not_ours"], "fixture.world.json")).expect("a manifest");
    let missing = seed::run(&first, &folder.join("missing.json"), None, "test").await;
    assert!(
        matches!(&missing, Err(SeedError::Vocabulary { component, .. }) if component == "template.not_ours"),
        "{missing:?}"
    );

    let refused_scope = path(&format!("template/site/{}", unique("refused")));
    let bad = json!({ "proto_version": "0.7.0", "doc": { "id": DocId::mint().as_str(), "kind": "world" }, "entities": [
        { "derivation": { "generator": DeclaredId::mint().to_string(), "key": "cell:0,0" }, "components": { "charging": {} } }
    ] });
    fs::write(folder.join("bad.world.json"), bad.to_string()).expect("a document");
    let refusing = json!({ "v": 1, "vocabulary": [], "entries": [{ "scope": refused_scope.to_string(), "document": "bad.world.json" }] });
    fs::write(folder.join("refused.json"), refusing.to_string()).expect("a manifest");
    let refused = seed::run(&first, &folder.join("refused.json"), None, "test").await;
    assert!(matches!(&refused, Err(SeedError::Entry { scope, .. }) if *scope == refused_scope), "{refused:?}");
    let _ = fs::remove_dir_all(&folder);
}
