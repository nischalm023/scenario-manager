//! Within `wps-client`, `wps` is a world-proto-server as this client speaks to it: one method per route it calls
//! (spec 04 section 6, Consumes), answered over HTTP by `http` and in memory by `mock`, so everything built on it
//! (check, writes and their resends, the export's closure check, the caches) is one code path, tested against the
//! mock.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use futures_util::Stream;
use serde_json::Value;
use studio_protocol::{EntityId, ScopePath, Seq};

use crate::error::WpsError;
use crate::value::LogEntry;
use crate::wire::{
    EntityQuery, ExportFlags, RawExport, Resolution, WireApplied, WireChangeset, WireComponent, WireDefinition,
    WireEntity, WireEvent, WireGenerator, WireGenerators, WireImport, WireListed, WireProtocol, WireReferrer,
    WireScope, WireTrail,
};

/// How a changeset's sending failed: WPS refused it, so nothing was written; WPS said Postgres ended the write and
/// nothing was written, which is resent once; or no answer came, so whether it landed is unknown.
#[derive(Debug, Clone, PartialEq)]
pub enum PostFailure {
    Answered(WpsError),
    EndedByPostgres { text: String },
    Lost { detail: String },
}

/// A scope's server-sent events, as long as the connection lasts.
pub type EventStream = Pin<Box<dyn Stream<Item = Result<WireEvent, WpsError>> + Send>>;

/// One world-proto-server.
pub trait Wps: Send + Sync + 'static {
    /// `GET /healthz`: the server's own version word.
    fn healthz(&self) -> impl Future<Output = Result<String, WpsError>> + Send;
    fn protocol(&self) -> impl Future<Output = Result<WireProtocol, WpsError>> + Send;
    fn protocol_defs(&self) -> impl Future<Output = Result<Value, WpsError>> + Send;
    fn scopes(&self) -> impl Future<Output = Result<Vec<WireScope>, WpsError>> + Send;
    /// `GET /scopes?under=`: every scope at or below `under`, read by one statement (scopes phase 3).
    fn scopes_under(&self, under: &ScopePath) -> impl Future<Output = Result<Vec<WireScope>, WpsError>> + Send;
    fn create_scope(
        &self,
        path: &ScopePath,
        boundary: bool,
    ) -> impl Future<Output = Result<WireScope, WpsError>> + Send;
    fn export_scope(
        &self,
        scope: &ScopePath,
        at: Option<Seq>,
    ) -> impl Future<Output = Result<RawExport, WpsError>> + Send;
    /// `GET /scopes/<scope>/-/export` with `flags` (Agent Studio's ruling D185), every scope a reference at head from
    /// the requested scopes can reach read at the changeset `at` names (one `at=<scope>@<changeset>` each, or none to
    /// read each at its head): one file holding what the flags ask for, whose `scopes` says what it holds. `flags` sets
    /// at least one flag, since with none WPS answers the plain export, `export_scope`'s.
    fn export_flagged(
        &self,
        scope: &ScopePath,
        flags: ExportFlags,
        at: &BTreeMap<ScopePath, Seq>,
    ) -> impl Future<Output = Result<Bytes, WpsError>> + Send;
    fn entity(
        &self,
        scope: &ScopePath,
        id: &EntityId,
        at: Option<Seq>,
    ) -> impl Future<Output = Result<WireEntity, WpsError>> + Send;
    /// `None` when the scope never held the id.
    fn trail(
        &self,
        scope: &ScopePath,
        id: &EntityId,
    ) -> impl Future<Output = Result<Option<WireTrail>, WpsError>> + Send;
    fn entities(
        &self,
        scope: &ScopePath,
        query: &EntityQuery,
    ) -> impl Future<Output = Result<(Vec<WireListed>, Option<String>), WpsError>> + Send;
    fn referrers(
        &self,
        scope: &ScopePath,
        id: &EntityId,
        at: Option<Seq>,
        after: Option<String>,
    ) -> impl Future<Output = Result<(Vec<WireReferrer>, Option<String>), WpsError>> + Send;
    fn resolve(&self, scope: &ScopePath, reference: &str) -> impl Future<Output = Result<Resolution, WpsError>> + Send;
    /// The log, newest first, of changesets before `before`.
    fn changesets(
        &self,
        scope: &ScopePath,
        before: Option<Seq>,
        limit: u16,
    ) -> impl Future<Output = Result<Vec<LogEntry>, WpsError>> + Send;
    /// The changeset of `scope` stored under an operation identifier, if any (`?operation=`, the first cut).
    fn changeset_named(
        &self,
        scope: &ScopePath,
        operation: &str,
    ) -> impl Future<Output = Result<Option<LogEntry>, WpsError>> + Send;
    fn post_changeset(
        &self,
        changeset: &WireChangeset,
    ) -> impl Future<Output = Result<WireApplied, PostFailure>> + Send;
    fn import(
        &self,
        scope: &ScopePath,
        base: Seq,
        document: &Value,
    ) -> impl Future<Output = Result<WireImport, WpsError>> + Send;
    fn components(&self) -> impl Future<Output = Result<Vec<WireDefinition>, WpsError>> + Send;
    /// `None` when no component of that name is defined.
    fn component(&self, name: &str) -> impl Future<Output = Result<Option<WireComponent>, WpsError>> + Send;
    fn generators(&self) -> impl Future<Output = Result<WireGenerators, WpsError>> + Send;
    fn generator(&self, name: &str) -> impl Future<Output = Result<WireGenerator, WpsError>> + Send;
    fn subscribe(&self, scope: &ScopePath) -> impl Future<Output = Result<EventStream, WpsError>> + Send;
}
