//! Within `wps-client`, `refusal` reads world-proto-server's answers that are not a success (spec 04 section 3.8).
//! Every answer of 400 or above that has a body is one refusal object, `{ "error", "code", "details" }` (world-proto
//! b9d82a8; the probe refuses a server without `structured_refusals`): a code that keeps its meaning, the facts under
//! keys never removed or renamed, and a sentence for people that any commit may reword. So a refusal is told apart by
//! its code and placed by its details, and its sentence is kept verbatim for display alone; a code or a detail key
//! this client does not know is ignored, and the status then says which kind the refusal is. An answer without the
//! object is no refusal of WPS's: hyper's own, to a request it could not read, a proxy's, or a fault on that side.

use semver::Version;
use serde_json::Value;
use studio_protocol::{EntityId, ScopePath, Seq};

use crate::error::{Problem, ProblemKind, WpsError};
use crate::value::GeneratorSet;
use crate::wire::WireRanGenerator;

/// Which route answered, where a code means more there: a missing parent is this client's own fault where it
/// creates a scope, since it creates parents first, the resolve route's refusals say what they say of the
/// target of the reference it was asked about, and the export route's say why the site it was asked for does not
/// close or why the changesets it was asked at name no cut of it, codes only its flagged export answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    CreateScope,
    Resolve,
    Export,
    Other,
}

/// What a refusal says: an error; WPS's word that Postgres ended a write and nothing was written, which the caller
/// resends once (spec 04 section 3.9); or its word that the generators a changeset would run differ from the set it
/// states, each difference in `details`, which the sender reads against that set (`generators_moved`).
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    Error(WpsError),
    EndedByPostgres { text: String },
    GeneratorsMoved { text: String, details: Value },
}

/// The largest changeset body WPS takes (`wp:server/server/src/api/mod.rs:232-242`).
pub const BODY_LIMIT: u64 = 64 * 1024 * 1024;

/// WPS's refusal object: its code, its facts and its sentence.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub code: String,
    pub details: Value,
    pub text: String,
}

impl Refusal {
    /// The refusal object `body` holds, or `None` for any other body: a proxy's page, hyper's own answer, nothing.
    pub fn of(body: &[u8]) -> Option<Self> {
        let json: Value = serde_json::from_slice(body).ok()?;
        let text = json.get("error")?.as_str()?.to_owned();
        let code = json.get("code")?.as_str()?.to_owned();
        let details = json.get("details").filter(|details| details.is_object())?.clone();
        Some(Self { code, details, text })
    }
}

/// What the resolve route answers in a resolution's place, read by its code (`wp:server/DESIGN.md` section 10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    /// `unknown_scope`: the referring scope does not exist.
    NoReferrer,
    /// `resolve_undeclared`: no scope has declared the reference's root.
    Undeclared,
    /// The scope that owns the target, with the problem the reference has there: none for a pin-rule refusal
    /// (`resolve_pin_within_unit`, `resolve_moving_across_units`), which the caller's own rule words; a miss
    /// otherwise: nothing live at head or at the pin, a pin past the owner's head, a tag the owner does not hold.
    Owned { owner: ScopePath, problem: Option<ProblemKind> },
}

/// A resolve route's refusal as what it says of the target, with its sentence; `None` for an error that says nothing
/// of it (the server unavailable, a malformed request), which the caller passes on.
pub fn unresolved(error: &WpsError) -> Option<(Unresolved, String)> {
    match error {
        WpsError::Unresolvable { unresolved, text } => Some((unresolved.clone(), text.clone())),
        _ => None,
    }
}

/// The sentence of an answer's body, for display and the log: its `error`, or the body itself.
pub fn sentence_of(body: &[u8]) -> String {
    match serde_json::from_slice::<Value>(body) {
        Ok(json) => json.get("error").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| json.to_string()),
        Err(_) => String::from_utf8_lossy(body).into_owned(),
    }
}

/// Read a refused answer of `route`: its status and body, `sent` being the bytes of the request it refuses.
pub fn read(route: RouteKind, status: u16, body: &[u8], sent: u64) -> Reading {
    let Some(Refusal { code, details, text }) = Refusal::of(body) else {
        let text = format!("{status} without world-proto-server's refusal object: {}", sentence_of(body));
        return Reading::Error(match status {
            502..=504 => WpsError::Unavailable { attempts: 1, last: text },
            500.. => WpsError::ServerFault { status, text },
            _ => WpsError::Invalid { text },
        });
    };
    if route == RouteKind::Resolve
        && let Some(unresolved) = unresolved_by(&code, &details)
    {
        return Reading::Error(WpsError::Unresolvable { unresolved, text });
    }
    let fault = |text: String| WpsError::ServerFault { status, text };
    let error = match (route, code.as_str()) {
        (_, "ended_by_postgres") => return Reading::EndedByPostgres { text },
        (_, "generators_moved") => return Reading::GeneratorsMoved { text, details },
        (_, "stale_base") => match stale(&details) {
            moved if moved.is_empty() => fault(text),
            moved => WpsError::HeadMoved { moved },
        },
        (_, "operation_reused") => operation_reused(&details).unwrap_or_else(|| fault(text)),
        (_, "registry_kept_moving") => match scope_at(&details, "scope") {
            Some(scope) => WpsError::RegistryMoved { scope, text },
            None => fault(text),
        },
        (_, "scope_exists") => match scope_at(&details, "scope") {
            Some(scope) => WpsError::ScopeExists { scope, head: Seq::new(0) },
            None => fault(text),
        },
        (RouteKind::CreateScope, "parent_missing") => WpsError::Invalid { text },
        (RouteKind::Export, "reached_twice" | "held_twice" | "closure_too_large") => {
            let kind = match code.as_str() {
                "reached_twice" => ProblemKind::PinnedTwice,
                "held_twice" => ProblemKind::HeldTwice,
                _ => ProblemKind::ClosureTooLarge,
            };
            let mut problem = Problem::new(kind, text);
            if let Some(scope) = scope_at(&details, "scope") {
                problem = problem.in_scope(&scope);
            }
            if let Some(entity) = details.get("entity").and_then(Value::as_str).and_then(|id| id.parse().ok()) {
                problem = problem.on(entity);
            }
            WpsError::ExportNotClosed { problems: vec![problem] }
        }
        (RouteKind::Export, "at_out_of_reach" | "at_left_out" | "at_beyond_head") => {
            let left_out = details.get("scopes").and_then(Value::as_array).and_then(|scopes| scopes.first());
            match scope_at(&details, "scope").or_else(|| left_out?.as_str()?.parse().ok()) {
                Some(scope) => WpsError::PinUnresolved { scope, text },
                None => fault(text),
            }
        }
        _ => match status {
            404 => WpsError::NotFound { text },
            413 => {
                let limit = details.get("limit").and_then(Value::as_u64).unwrap_or(BODY_LIMIT);
                WpsError::TooLarge { bytes: sent, limit }
            }
            422 => WpsError::Refused { problem: Box::new(problem_of(&code, &details, text)) },
            502..=504 => WpsError::Unavailable { attempts: 1, last: format!("{status}: {text}") },
            409 | 500.. => fault(text),
            _ => WpsError::Invalid { text },
        },
    };
    Reading::Error(error)
}

fn scope_at(details: &Value, key: &str) -> Option<ScopePath> {
    details.get(key).and_then(Value::as_str).and_then(|text| text.parse().ok())
}

/// `stale_base`'s `stale`: every scope whose head passed the base the changeset states, with both changesets.
fn stale(details: &Value) -> Vec<(ScopePath, Seq, Seq)> {
    let stale = details.get("stale").and_then(Value::as_array).into_iter().flatten();
    stale
        .filter_map(|one| {
            let (base, head) = (one.get("base")?.as_u64()?, one.get("head")?.as_u64()?);
            Some((scope_at(one, "scope")?, Seq::new(base), Seq::new(head)))
        })
        .collect()
}

/// `operation_reused`: the identifier, the scope its stored changeset was initiated on, and each scope that wrote.
fn operation_reused(details: &Value) -> Option<WpsError> {
    let operation = details.get("operation")?.as_str()?.to_owned();
    let named = details.get("named").and_then(Value::as_array).into_iter().flatten();
    let named = named.filter_map(|one| Some((scope_at(one, "scope")?, Seq::new(one.get("changeset")?.as_u64()?))));
    Some(WpsError::OperationReused { operation, initiated: scope_at(details, "initiated")?, named: named.collect() })
}

fn unresolved_by(code: &str, details: &Value) -> Option<Unresolved> {
    let owned = |problem: Option<ProblemKind>| Some(Unresolved::Owned { owner: scope_at(details, "owner")?, problem });
    match code {
        "unknown_scope" => Some(Unresolved::NoReferrer),
        "resolve_undeclared" => Some(Unresolved::Undeclared),
        "resolve_pin_within_unit" | "resolve_moving_across_units" => owned(None),
        "resolve_dangling" => owned(Some(ProblemKind::Dangling)),
        "resolve_tag_not_held" => owned(Some(ProblemKind::TagNotHeld)),
        "resolve_pinned_dangling" => owned(Some(ProblemKind::PinnedDangling)),
        "resolve_pin_beyond_head" => owned(Some(ProblemKind::PinBeyondHead)),
        _ => None,
    }
}

/// A 422's problem: its kind named as its code is, `Other` for a code this client does not know, placed where its
/// details say: where a reference stands (`referrer`), the first identity a deletion would leave named (`targets`),
/// else the details' own scope, entity, component and path; and the version a payload was validated against.
fn problem_of(code: &str, details: &Value, text: String) -> Problem {
    let kind = serde_json::from_value(Value::String(code.replace('_', "-"))).unwrap_or(ProblemKind::Other);
    let first_target = details.get("targets").and_then(Value::as_array).and_then(|targets| targets.first());
    let place = details.get("referrer").filter(|referrer| referrer.is_object()).or(first_target).unwrap_or(details);
    let text_at = |key: &str| place.get(key).and_then(Value::as_str);
    Problem {
        kind,
        scope: scope_at(place, "scope"),
        entity: ["entity", "target", "written"].into_iter().find_map(|key| text_at(key)?.parse::<EntityId>().ok()),
        component: text_at("component").map(str::to_owned),
        path: text_at("path").map(str::to_owned),
        version: details.get("version").and_then(Value::as_str).map(str::to_owned),
        text,
    }
}

/// `generators_moved` read from its `differences` against the set the changeset stated: `found` is that set with
/// each difference applied, a generator that would not run removed and one that runs at another version or module,
/// or unstated, put at what runs, so `GeneratorsMoved` says what runs instead.
pub fn generators_moved(details: &Value, stated: &[WireRanGenerator]) -> WpsError {
    let version = |text: &str| Version::parse(text).ok();
    let expected: GeneratorSet = GeneratorSet(
        stated
            .iter()
            .filter_map(|one| Some((one.generator.clone(), (version(&one.version)?, one.digest.clone()))))
            .collect(),
    );
    let mut found = expected.clone();
    for difference in details.get("differences").and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = difference.get("generator").and_then(Value::as_str) else { continue };
        let runs = difference.get("runs").filter(|runs| !runs.is_null());
        let pinned = runs.and_then(|runs| {
            let at = version(runs.get("version")?.as_str()?)?;
            Some((at, runs.get("digest")?.as_str()?.to_owned()))
        });
        match (runs, pinned) {
            (None, _) => {
                found.0.remove(name);
            }
            (Some(_), Some(pinned)) => {
                found.0.insert(name.to_owned(), pinned);
            }
            (Some(_), None) => {}
        }
    }
    WpsError::GeneratorsMoved { expected, found }
}
