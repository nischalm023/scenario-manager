//! `site-template` is Agent Studio's template evaluation engine (spec 05 section 3.6): it evaluates a site template,
//! read whole at one changeset per scope, at given knob values into the entities and records a made site declares,
//! each with its source key, with the problems, the notes and the census, and without I/O. The API links it and
//! runs it on its compute pool; placements expand through foundry's own stamp instantiation, so an evaluation, its
//! floor and a run share one implementation of stamping.

mod count;
mod evaluate;
pub mod evaluation;
pub mod expression;
pub mod knob;
mod layout;
pub mod read;
mod references;
pub mod source_key;
mod template;
mod values;

pub use count::{Count, InstanceIndex};
pub use evaluate::{IdSource, Mint, Prior, evaluate};
pub use evaluation::{
    Census, Declaration, EngineStamp, Evaluation, Note, PROTO_VERSION, Problem, ProblemCode, RelativeScope, ScopePlan,
    WorldDocument,
};
pub use knob::{KnobDecl, KnobDefault, KnobForm, KnobKey, KnobUnit, KnobValue, KnobValues, Outside, Severity};
pub use read::{
    DEFS_URN, DeclaredEntity, DocId, Prologue, Record, RefAnnotation, RegisteredComponent, RegistrySnapshot,
    ScopeSnapshot, SnapshotFault, TemplateRead,
};
pub use source_key::SourceKey;
