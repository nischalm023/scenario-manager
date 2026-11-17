//! Agent Studio's world-proto-server client (spec 04, the WPS client): the one door through which the API, the seed
//! and the runner read and write the world graph WPS keeps (ruling D35). It turns WPS's routes into typed reads at a
//! pin, one atomic changeset per draft with a base for every scope it writes or reads, a closure-checked export of a
//! whole site, and typed errors, and maps the final scope model onto whatever WPS has landed (section 3.12) so no
//! consumer has to.

pub mod apply;
pub mod cache;
pub mod capability;
pub mod check;
pub mod client;
pub mod compute;
pub mod draft;
pub mod error;
pub mod events;
pub mod export;
pub mod http;
pub mod kinship;
pub mod mock;
pub mod refusal;
pub mod routes;
pub mod schema;
pub mod seed;
pub mod site;
pub mod snapshot;
pub mod value;
pub mod walker;
pub mod wire;
pub mod wps;

pub use apply::NotLanded;
pub use client::{CheckedBy, Client, ClientConfig, Preview, WorldStore, http_from_env};
pub use compute::{BlockingCompute, Compute, ComputeBusy};
pub use draft::{Applied, ChangesetDraft, DraftBuilder, ImportProposal, NewScope, Op, ScopedOp};
pub use error::{ApiProblem, Problem, ProblemKind, WpsError};
pub use events::ScopeEvents;
pub use http::{HttpConfig, HttpWps};
pub use mock::MockWps;
pub use snapshot::{
    Component, CutExport, DeclaredEntity, DuplicateKey, EntityReading, Record, ScopeEvent, ScopeSnapshot, UnitSnapshot,
    Walked,
};
pub use value::{
    ArchivedRecord, Author, Capabilities, Capability, CommitPlace, CreatedScope, DerivedListing, DocId, Exactness,
    ExternalValue, GenerationRan, GeneratorSet, LogEntry, OpTally, Page, Prologue, Reference, Referrer, ScopeListing,
    UnitCut, Version,
};
