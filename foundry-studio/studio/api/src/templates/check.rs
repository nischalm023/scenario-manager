//! Within `templates`, `check` is spec 05 section 4.3's step 6.12 as the client runs it (spec 04 section 3.4): the site
//! changeset a draft's evaluation would make, checked by the client before the evaluation says ready (schemas,
//! references and pins, the body's size), so that Create site meets a checked changeset and a predictable refusal
//! shows as the evaluation's problem before anyone approves it (LEAD-DECISIONS entry 54). The site's own path is named
//! only at Create site, so the check stands its four scopes at a path of its own.

use std::fmt::Display;
use std::sync::Arc;

use rustc_hash::FxHashMap;
use studio_protocol::{DeclaredId, EvaluationProblem, Severity, SitePath};
use tokio::runtime::Handle;
use tokio::select;
use tokio::sync::watch;
use wps_client::wps::Wps;
use wps_client::{Author, Problem as WpsProblem, WorldStore, WpsError};

use super::evaluation::pooled;
use super::plan::PlanFile;
use crate::GateState;

/// Where the client checks the site an evaluation would make: a site's root, of a customer whose slug says what it
/// is for.
const CHECKED_AT: &str = "customer/agent-studio-check/site/evaluation";
const CHECKED_MESSAGE: &str = "Agent Studio: the client's check of an evaluation's would-be site";

/// The check of the site `plan` would make, named `name`: its four scopes at the stand-in path held against their
/// empty base on the compute pool, its few reads of world-proto-server waiting there, and given up there once nobody
/// awaits it; each problem an error of the evaluation's under the source key `ids` gives the entity it names, and a
/// check that cannot run one too.
pub async fn checked<S: Wps>(
    gate: &GateState<S>,
    plan: &[u8],
    ids: &[(String, DeclaredId)],
    name: &str,
) -> Vec<EvaluationProblem> {
    let error = |code: &str, reference: Option<String>, text: String| EvaluationProblem {
        severity: Severity::Error,
        code: code.into(),
        knob: None,
        ref_: reference,
        text,
    };
    let unchecked = |code: &str, fault: &dyn Display| {
        let text = format!("Error: The site these knobs make could not be checked: {fault}");
        vec![error(code, None, text)]
    };
    let file: PlanFile = match serde_json::from_slice(plan) {
        Ok(file) => file,
        Err(fault) => return unchecked("internal", &fault),
    };
    let generators = file.generators(&gate.world.registry().snapshot());
    let (site, author) = (CHECKED_AT.parse::<SitePath>(), Author::new("Agent Studio"));
    let plan = match (site, author) {
        (Ok(site), Ok(author)) => file.site_plan(&site, name, author, CHECKED_MESSAGE.into(), "check", generators),
        (Err(fault), _) => return unchecked("internal", &fault),
        (_, Err(fault)) => return unchecked("internal", &fault),
    };
    let (world, handle) = (gate.world.clone(), Handle::current());
    // The sender lives as long as this check is awaited: an evaluation that drops the check (its floor failed) drops
    // it too, and the pool's thread stops waiting on world-proto-server instead of holding a worker for nobody.
    let (_awaited, mut dropped) = watch::channel(());
    let checking = plan.map(|plan| (Arc::new(plan.site_alone()), Arc::new(plan.empty_base())));
    let answer = match checking {
        Ok((draft, base)) => {
            let work = move || {
                handle.block_on(async move {
                    select! {
                        answer = world.check(&draft, &base) => answer,
                        _ = dropped.changed() => Err(WpsError::Invalid { text: "the check was abandoned".into() }),
                    }
                })
            };
            pooled(&*gate.app.compute, work).await
        }
        Err(fault) => Err(fault),
    };
    let problems = match answer {
        Ok(Ok(problems)) => problems,
        Ok(Err(fault)) | Err(fault) => return unchecked(fault.api_problem().type_, &fault),
    };
    let keys: FxHashMap<DeclaredId, &str> = ids.iter().map(|(key, id)| (*id, key.as_str())).collect();
    let key_of = |problem: &WpsProblem| {
        let entity = problem.entity.as_ref()?;
        keys.get(&entity.declared()).map(|key| (*key).to_owned()).or_else(|| Some(entity.to_string()))
    };
    let would = |problem: &WpsProblem| format!("Error: The site these knobs make would be refused: {}", problem.text);
    problems.iter().map(|problem| error(&problem.kind.to_string(), key_of(problem), would(problem))).collect()
}
