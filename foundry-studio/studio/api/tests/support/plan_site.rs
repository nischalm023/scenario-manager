//! A site made from a shell template for what draws one (brief I1.12): the template evaluated by the engine as the
//! API runs it (`studio_api::templates::compute`), and its plan written into the site's scopes one scope at a time,
//! the map first and the root's prologue, naming the site and its primary frame, last, so it is taken by a
//! world-proto-server of any phase: the 60aa424 a dev stack runs takes no changeset across scopes, which Create site
//! now writes (spec 05 section 4.4). The tests read it as `support::plan_site`; `examples/made_site.rs` runs it against
//! the world-proto-server a dev stack started, for the browser tests.

use std::collections::BTreeMap;

use rustc_hash::FxHashMap;
use site_template::{DocId as TemplateDocId, Prior};
use studio_api::templates::{Computed, Inputs, Kept, PlanFile, compute, read};
use studio_protocol::{BranchName, KnobValue, Pin, ScopePath, Seq};
use wps_client::draft::Op;
use wps_client::site::{SUB_SCOPES, ensure_prefixes};
use wps_client::wire::WireChangeset;
use wps_client::wps::Wps;
use wps_client::{Client, DocId, Prologue, WorldStore};

/// `template/site/{shell}` at its head evaluated with `knobs` over its defaults, as the API computes an evaluation.
pub async fn evaluated<S: Wps>(world: &Client<S>, shell: &str, knobs: &[(&str, KnobValue)], name: &str) -> Computed {
    let main = Pin::Branch(BranchName::main());
    let template: ScopePath = format!("template/site/{shell}").parse().expect("a template's root");
    let at = world.resolve_cut(&template, &main).await.expect("the template's cut");
    let read = read(&Kept::default(), world, &template, &at.seqs).await.expect("the template reads");
    // Only the knobs set: every other follows its default, derived at each evaluation (D120).
    let values: BTreeMap<String, KnobValue> = knobs.iter().map(|(key, value)| ((*key).to_owned(), *value)).collect();
    let generators = world.registry().snapshot().generators.clone();
    let prior = Prior::new(TemplateDocId::mint());
    let computed = compute(&Inputs { read, knobs: values, prior, name: name.to_owned(), generators });
    assert!(computed.ready, "the {shell} evaluates and compiles: {:?}", computed.problems);
    computed
}

/// Make `root`, named `name`, from `template/site/{shell}` at its head with `knobs` over its defaults; the site's
/// scopes and the changeset each landed at.
pub async fn made_site<S: Wps>(
    world: &Client<S>,
    shell: &str,
    knobs: &[(&str, KnobValue)],
    root: &str,
    name: &str,
) -> (ScopePath, BTreeMap<ScopePath, Seq>) {
    let computed = evaluated(world, shell, knobs, name).await;
    let plan: PlanFile = serde_json::from_slice(&computed.plan).expect("the plan reads back");

    let root: ScopePath = root.parse().expect("a site's root");
    ensure_prefixes(world, &root).await.expect("the root's prefixes");
    world.ensure_scope(&root, true).await.expect("the root");
    let mut ops: FxHashMap<ScopePath, Vec<Op>> = FxHashMap::default();
    for sub in SUB_SCOPES {
        let below = root.join(&sub.parse().expect("a slug")).expect("a sub-scope");
        world.ensure_scope(&below, false).await.expect("a sub-scope");
        ops.insert(below, Vec::new());
    }
    for made in &plan.scopes {
        let scope = match made.scope.as_str() {
            "." => root.clone(),
            below => root.join(&below.parse().expect("a slug")).expect("a scope of the site"),
        };
        let held = ops.entry(scope).or_default();
        for declared in &made.declarations {
            held.push(Op::Declare {
                id: declared.id,
                name: declared.name.clone(),
                components: declared.components.clone(),
            });
        }
        for record in &made.records {
            held.push(match &record.components {
                Some(components) => Op::Override { target: record.target.clone(), components: components.clone() },
                None => Op::Suppress { target: record.target.clone() },
            });
        }
    }
    let prologue = Prologue {
        doc: plan.doc.parse::<DocId>().expect("the plan's document id"),
        kind: Some("world".to_owned()),
        name: Some(name.to_owned()),
        primary_frame: plan.primary_frame.clone(),
        depends_on: Vec::new(),
    };
    // The map before what refers to it, the fleet before the flow whose segments name its cohorts, the root last.
    let order = ["map", "fleet", "flow"].map(|sub| root.join(&sub.parse().expect("a slug")).expect("a sub-scope"));
    // ordered: examples/made_site.rs prints each scope's changeset in path order.
    let mut seqs = BTreeMap::new();
    for scope in order.iter().chain([&root]) {
        let base = world.tree().await.expect("the scope tree").head(scope).expect("the scope exists");
        let staged = ops.remove(scope).unwrap_or_default();
        let message = format!("Made {name} from template/site/{shell} (tests)");
        let mut changeset = WireChangeset::of_one_scope(scope.clone(), base, "tests", &message, staged);
        changeset.prologue = (scope == &root).then(|| prologue.clone());
        let landed = world.wps().post_changeset(&changeset).await.expect("the stage lands");
        seqs.insert(scope.clone(), landed.seq.expect("the scope's new changeset"));
    }
    (root, seqs)
}
