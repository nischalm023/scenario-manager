//! Within `runs`, `findings` is spec 08 section 3.5's comparison of a runner's manifest with what the check recorded
//! for its run, which exists so that a number is never read as the check's when the runner executed something else,
//! while a run whose read merely turned inexact, or whose other compiler gave the same plan, raises no warning. Three
//! kinds, apart: `plan-differs` (the compile digest, the world's or the workload's), `read-differs` (the changesets
//! read or their exactness) and `compiler-differs` (the compiler's version), each with spec 08 section 7's sentence.
//! Historical runs may have null compile digests, which compare equal. Findings do not stop a run.

use std::collections::BTreeMap;

use studio_protocol::{
    AttemptNumber, Expectation, Finding, FindingKind, FindingPart, FindingSide, Manifest, ScopePath, Seq, Ulid,
};

/// What the check recorded for one run: its expectation, the plan's reads and whether the plan's cut was exact.
pub struct Checked<'a> {
    pub expected: &'a Expectation,
    pub reads: &'a BTreeMap<ScopePath, Seq>,
    pub exact: bool,
}

/// Every finding `manifest` raises against `checked`, at most one of each kind, `plan-differs` first.
pub fn findings(run: &Ulid, attempt: AttemptNumber, checked: &Checked<'_>, manifest: &Manifest) -> Vec<Finding> {
    let expected = FindingSide {
        compile_sha256: checked.expected.compile_sha256.clone(),
        world_sha256: checked.expected.world_sha256.clone(),
        workload_sha256: checked.expected.workload_sha256.clone(),
        compiler: checked.expected.compiler.clone(),
        reads: checked.reads.clone(),
        exact: checked.exact,
        exact_reason: None,
    };
    let got = FindingSide {
        compile_sha256: manifest.compile.digest.clone(),
        world_sha256: manifest.world.sha256.clone(),
        workload_sha256: manifest.workload.sha256.clone(),
        compiler: manifest.compile.compiler.clone(),
        reads: manifest.reads.clone(),
        exact: manifest.world.exact,
        exact_reason: manifest.world.exact_reason.clone(),
    };
    let lead = format!("Run {}, attempt {}", &run.as_str()[..8], attempt.0);
    let mut found = Vec::new();
    let mut plan = Vec::new();
    if got.compile_sha256 != expected.compile_sha256 {
        plan.push((
            FindingPart::Compile,
            format!(
                "compiled another flow plan than the check: its {} against the check's {}",
                short(got.compile_sha256.as_deref()),
                short(expected.compile_sha256.as_deref())
            ),
        ));
    }
    if got.world_sha256 != expected.world_sha256 {
        plan.push((
            FindingPart::World,
            "loaded another world than the check: labels or the prologue moved since it".into(),
        ));
    }
    if got.workload_sha256 != expected.workload_sha256 {
        plan.push((FindingPart::Workload, "got another generated workload than the check".into()));
    }
    let plan_differs = !plan.is_empty();
    if plan_differs {
        let words: Vec<&str> = plan.iter().map(|(_, words)| words.as_str()).collect();
        let sentence = format!("{lead}, {}; the run goes on.", words.join(", and "));
        found.push(finding(FindingKind::PlanDiffers, plan, &expected, &got, sentence));
    }
    let mut read = Vec::new();
    if got.exact != expected.exact {
        let why = got.exact_reason.clone().unwrap_or_else(|| "its read was exact where the check's was not".to_owned());
        read.push((
            FindingPart::Exact,
            format!("read below a moved head, so its labels and prologue are the head's: {why}"),
        ));
    }
    let held = &manifest.world.scopes;
    if let Some((scope, theirs, ours)) =
        first_difference(&got.reads, &expected.reads).or_else(|| first_difference(held, &expected.reads))
    {
        let name = scope.as_str().rsplit('/').next().unwrap_or(scope.as_str()).to_owned();
        let changeset = |seq: Option<Seq>| seq.map_or_else(|| "nothing".to_owned(), |seq| format!("changeset {seq}"));
        read.push((
            FindingPart::Reads,
            format!("read {name} {} where the check read {}", changeset(theirs), changeset(ours)),
        ));
    }
    if !read.is_empty() {
        let words: Vec<&str> = read.iter().map(|(_, words)| words.as_str()).collect();
        let sentence = format!("{lead}, {}.", words.join("; and "));
        found.push(finding(FindingKind::ReadDiffers, read, &expected, &got, sentence));
    }
    if got.compiler != expected.compiler {
        let same = if plan_differs { "" } else { " to the same plan" };
        let sentence =
            format!("{lead}, used compiler {} where the check used {}{same}.", got.compiler, expected.compiler);
        let parts = vec![(FindingPart::Compiler, String::new())];
        found.push(finding(FindingKind::CompilerDiffers, parts, &expected, &got, sentence));
    }
    found
}

fn finding(
    kind: FindingKind,
    parts: Vec<(FindingPart, String)>,
    expected: &FindingSide,
    got: &FindingSide,
    sentence: String,
) -> Finding {
    Finding {
        kind,
        differs: parts.into_iter().map(|(part, _)| part).collect(),
        expected: expected.clone(),
        got: got.clone(),
        sentence,
    }
}

/// The first scope, in path order, read at another changeset by one side than the other.
fn first_difference(
    got: &BTreeMap<ScopePath, Seq>,
    expected: &BTreeMap<ScopePath, Seq>,
) -> Option<(ScopePath, Option<Seq>, Option<Seq>)> {
    let mut scopes: Vec<&ScopePath> = got.keys().chain(expected.keys()).collect();
    scopes.sort();
    scopes.dedup();
    scopes
        .into_iter()
        .map(|scope| (scope.clone(), got.get(scope).copied(), expected.get(scope).copied()))
        .find(|(_, theirs, ours)| theirs != ours)
}

/// A digest's first six hex digits, as section 7's sentences quote it.
fn short(digest: Option<&str>) -> String {
    digest.map_or_else(|| "none".to_owned(), |digest| format!("{}…", &digest[..digest.len().min(6)]))
}
