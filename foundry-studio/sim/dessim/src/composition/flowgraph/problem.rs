//! Within `flowgraph`, a `problem` is spec 01's problem item as the flow compiler raises it (spec 06 section 3.2,
//! the checks table): what is wrong by code, how badly, the flow entity and field it names, and one sentence the
//! panel and the agent show as written. The API adds the item's `scope` and `knob`, which the compiler cannot know.

use std::fmt::Display;

use serde::Serialize;
use site_model::ids::EntityId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// Section 3.2's codes in its table's order, then section 3.4's `not-sweepable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Code {
    SchemaUnknown,
    RefUnresolved,
    NodeEmpty,
    MemberUnfit,
    HoldsMismatch,
    OverlapExclusive,
    PortInvalid,
    EdgeUnrealized,
    ChainBroken,
    CustodyGap,
    ExecutorUnresolved,
    ExecutorCannotTake,
    ProcessIncomplete,
    GuardGap,
    GuardOverlap,
    AlternativeInvalid,
    WorkMissing,
    WorkDisagreesWithMap,
    UnsupportedValue,
    UnsupportedPattern,
    ConsumerNotReady,
    ConsumerConflict,
    NodeUnused,
    FlowEmpty,
    NotSweepable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub code: Code,
    pub severity: Severity,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<EntityId>,
    /// A JSON pointer into the entity, opening with its component's name: `/flow.node/work/pick_sim_ms`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub text: String,
}

impl Problem {
    pub fn error(code: Code, reference: &EntityId, field: Option<String>, text: impl Display) -> Self {
        Self {
            code,
            severity: Severity::Error,
            reference: Some(reference.clone()),
            field,
            text: format!("Error: {text}"),
        }
    }

    pub fn warning(code: Code, reference: Option<&EntityId>, field: Option<String>, text: impl Display) -> Self {
        let text = format!("Warning: {text}");
        Self { code, severity: Severity::Warning, reference: reference.cloned(), field, text }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Errors first, then by code, entity, field and sentence: one order for every compile of one input.
pub(super) fn sort(problems: &mut [Problem]) {
    problems.sort_by(|a, b| {
        (a.severity, a.code, &a.reference, &a.field, &a.text).cmp(&(
            b.severity,
            b.code,
            &b.reference,
            &b.field,
            &b.text,
        ))
    });
}
