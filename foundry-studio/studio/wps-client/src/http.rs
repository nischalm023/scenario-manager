//! Within `wps-client`, `http` is the `Wps` that speaks to a real world-proto-server: every request through the URLs
//! of `routes`, under spec 04 section 3.3's limits (4 exports, whole-site reads or writes and 8 requests in all at
//! once, the rest queued) and section 3.9's timeouts and retries. A read is retried on a failed connection, a reset
//! or a 502 to 504, three tries in all, and once after a 500, and its refusal is read by its code (`refusal`); a
//! changeset is sent once, and an unanswered one is reported lost, for a resend of the same body under its operation
//! identifier to settle, as is any answer to it that is neither a readable 201 nor WPS's own refusal object.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap};
use reqwest::{Client, Error, Method, RequestBuilder, Url};
use serde::de::DeserializeOwned;
use serde_json::Value;
use studio_protocol::{EntityId, ScopePath, Seq};
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::error::WpsError;
use crate::refusal::{self, Reading, Refusal, RouteKind};
use crate::routes::Routes;
use crate::value::LogEntry;
use crate::wire::{
    EntityQuery, ExportFlags, RawExport, Resolution, WireApplied, WireChangeset, WireComponent, WireDefinition,
    WireEntity, WireEvent, WireGenerator, WireGenerators, WireImport, WireListed, WireProtocol, WireReferrer,
    WireScope, WireTrail,
};
use crate::wps::{EventStream, PostFailure, Wps};

/// How many requests may be in flight to WPS at once; never fewer than one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Concurrency(usize);

impl Concurrency {
    pub fn new(requests: usize) -> Self {
        Self(requests.max(1))
    }

    pub fn get(self) -> usize {
        self.0
    }
}

/// How this client talks to one server: where, how long it waits, and how much it sends at once. Every timeout and
/// wait is wall clock.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub routes: Routes,
    pub read_timeout: Duration,
    pub export_timeout: Duration,
    pub write_timeout: Duration,
    pub heavy_at_once: Concurrency,
    pub requests_at_once: Concurrency,
    pub retry_waits: [Duration; 2],
}

impl HttpConfig {
    /// Spec 04's defaults: 30 s, 120 s and 180 s; 4 and 8 at once; waits of 0.5 s and 2 s between tries.
    pub fn new(routes: Routes) -> Self {
        Self {
            routes,
            read_timeout: Duration::from_secs(30),
            export_timeout: Duration::from_secs(120),
            write_timeout: Duration::from_secs(180),
            heavy_at_once: Concurrency::new(4),
            requests_at_once: Concurrency::new(8),
            retry_waits: [Duration::from_millis(500), Duration::from_secs(2)],
        }
    }
}

/// What a request weighs against the limits: an export, a whole-site read or a write is heavy.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Weight {
    Light,
    Heavy,
}

/// A world-proto-server over HTTP.
pub struct HttpWps {
    http: Client,
    config: HttpConfig,
    all: Semaphore,
    heavy: Semaphore,
    sent: AtomicU64,
}

/// An answer: status, headers and body.
struct Answer {
    status: u16,
    headers: HeaderMap,
    body: Bytes,
}

/// How a request that writes nothing ended past its retries: a success, or a refusal its caller reads by route.
enum Asked {
    Success(Answer),
    Refused { attempts: u8, answer: Answer },
}

/// A refused answer of `route` as this client's error, `sent` bytes having been sent.
fn refused(route: RouteKind, attempts: u8, answer: &Answer, sent: u64) -> WpsError {
    match refusal::read(route, answer.status, &answer.body, sent) {
        Reading::Error(WpsError::Unavailable { last, .. }) => WpsError::Unavailable { attempts, last },
        Reading::Error(error) => error,
        Reading::EndedByPostgres { text } => WpsError::Unavailable { attempts, last: text },
        Reading::GeneratorsMoved { text, .. } => WpsError::ServerFault { status: 409, text },
    }
}

impl HttpWps {
    pub fn new(config: HttpConfig) -> Result<Self, WpsError> {
        let http = Client::builder()
            .build()
            .map_err(|fault| WpsError::Invalid { text: format!("an HTTP client could not be built: {fault}") })?;
        let (all, heavy) = (Semaphore::new(config.requests_at_once.get()), Semaphore::new(config.heavy_at_once.get()));
        Ok(Self { http, config, all, heavy, sent: AtomicU64::new(0) })
    }

    pub fn routes(&self) -> &Routes {
        &self.config.routes
    }

    /// How many requests this client has sent WPS, retries included.
    pub fn requests_sent(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }

    async fn permits(&self, weight: Weight) -> (Option<SemaphorePermit<'_>>, SemaphorePermit<'_>) {
        let heavy = match weight {
            Weight::Heavy => Some(self.heavy.acquire().await.expect("the limit is never closed")),
            Weight::Light => None,
        };
        (heavy, self.all.acquire().await.expect("the limit is never closed"))
    }

    fn request(&self, method: Method, url: Url, timeout: Duration, body: Option<&Bytes>) -> RequestBuilder {
        let request = self.http.request(method, url).timeout(timeout);
        match body {
            Some(body) => request.header(CONTENT_TYPE, "application/json").body(body.clone()),
            None => request,
        }
    }

    /// Send once and read the whole answer, or say why no answer came.
    async fn once(&self, weight: Weight, request: RequestBuilder) -> Result<Answer, Error> {
        let _permits = self.permits(weight).await;
        self.sent.fetch_add(1, Ordering::Relaxed);
        let answer = request.send().await?;
        let (status, headers) = (answer.status().as_u16(), answer.headers().clone());
        let body = answer.bytes().await?;
        Ok(Answer { status, headers, body })
    }

    /// A request that writes nothing, or writes idempotently: retried as section 3.9 says.
    async fn ask(
        &self,
        method: Method,
        url: Url,
        weight: Weight,
        timeout: Duration,
        body: Option<&Bytes>,
    ) -> Result<Asked, WpsError> {
        let mut attempt = 0u8;
        loop {
            attempt += 1;
            let wait = self.config.retry_waits.get(usize::from(attempt) - 1).copied();
            let request = self.request(method.clone(), url.clone(), timeout, body);
            let last = match self.once(weight, request).await {
                Ok(answer) if (200..300).contains(&answer.status) => return Ok(Asked::Success(answer)),
                Ok(answer) => {
                    let retried = match answer.status {
                        502..=504 => attempt < 3,
                        500 => attempt < 2,
                        _ => false,
                    };
                    if !retried {
                        return Ok(Asked::Refused { attempts: attempt, answer });
                    }
                    format!("{} from {url}", answer.status)
                }
                Err(fault) if fault.is_timeout() => {
                    return Err(WpsError::Unavailable { attempts: attempt, last: format!("{url} timed out: {fault}") });
                }
                Err(fault) if attempt < 3 => format!("{url}: {fault}"),
                Err(fault) => return Err(WpsError::Unavailable { attempts: attempt, last: format!("{url}: {fault}") }),
            };
            match wait {
                Some(wait) => tokio::time::sleep(wait).await,
                None => return Err(WpsError::Unavailable { attempts: attempt, last }),
            }
        }
    }

    /// `ask`, its refusal read as `route`'s.
    async fn read(
        &self,
        route: RouteKind,
        method: Method,
        url: Url,
        weight: Weight,
        timeout: Duration,
        body: Option<Bytes>,
    ) -> Result<Answer, WpsError> {
        let sent = body.as_ref().map_or(0, |body| body.len() as u64);
        match self.ask(method, url, weight, timeout, body.as_ref()).await? {
            Asked::Success(answer) => Ok(answer),
            Asked::Refused { attempts, answer } => Err(refused(route, attempts, &answer, sent)),
        }
    }

    async fn get<T: DeserializeOwned>(&self, url: Url, weight: Weight) -> Result<T, WpsError> {
        let timeout = match weight {
            Weight::Light => self.config.read_timeout,
            Weight::Heavy => self.config.export_timeout,
        };
        let answer = self.read(RouteKind::Other, Method::GET, url.clone(), weight, timeout, None).await?;
        decode(&url, &answer.body)
    }

    /// `GET` that reads WPS's refusal `absent`, the route's word that nothing is there, as `None`.
    async fn get_or_none<T: DeserializeOwned>(&self, url: Url, absent: &str) -> Result<Option<T>, WpsError> {
        match self.ask(Method::GET, url.clone(), Weight::Light, self.config.read_timeout, None).await? {
            Asked::Success(answer) => decode(&url, &answer.body).map(Some),
            Asked::Refused { answer, .. }
                if Refusal::of(&answer.body).is_some_and(|refusal| refusal.code == absent) =>
            {
                Ok(None)
            }
            Asked::Refused { attempts, answer } => Err(refused(RouteKind::Other, attempts, &answer, 0)),
        }
    }
}

fn decode<T: DeserializeOwned>(url: &Url, body: &[u8]) -> Result<T, WpsError> {
    serde_json::from_slice(body)
        .map_err(|fault| WpsError::Invalid { text: format!("{url} answered what this client cannot read: {fault}") })
}

fn query(mut url: Url, pairs: &[(&str, Option<String>)]) -> Url {
    let stated: Vec<(&str, String)> =
        pairs.iter().filter_map(|(key, value)| value.clone().map(|value| (*key, value))).collect();
    if !stated.is_empty() {
        url.query_pairs_mut().extend_pairs(stated);
    }
    url
}

impl Wps for HttpWps {
    async fn healthz(&self) -> Result<String, WpsError> {
        let answer: Value = self.get(self.config.routes.healthz(), Weight::Light).await?;
        Ok(answer.get("server").and_then(Value::as_str).unwrap_or("unknown").to_owned())
    }

    async fn protocol(&self) -> Result<WireProtocol, WpsError> {
        self.get(self.config.routes.protocol(), Weight::Light).await
    }

    async fn protocol_defs(&self) -> Result<Value, WpsError> {
        self.get(self.config.routes.protocol_defs(), Weight::Light).await
    }

    async fn scopes(&self) -> Result<Vec<WireScope>, WpsError> {
        let answer: Value = self.get(self.config.routes.scopes(), Weight::Light).await?;
        let rows = answer.get("scopes").cloned().unwrap_or(Value::Null);
        serde_json::from_value(rows).map_err(|fault| WpsError::Invalid { text: format!("GET /scopes: {fault}") })
    }

    async fn scopes_under(&self, under: &ScopePath) -> Result<Vec<WireScope>, WpsError> {
        let answer: Value = self.get(self.config.routes.scopes_under(under), Weight::Light).await?;
        let rows = answer.get("scopes").cloned().unwrap_or(Value::Null);
        serde_json::from_value(rows)
            .map_err(|fault| WpsError::Invalid { text: format!("GET /scopes?under={under}: {fault}") })
    }

    async fn create_scope(&self, path: &ScopePath, boundary: bool) -> Result<WireScope, WpsError> {
        let body = Bytes::from(serde_json::json!({ "path": path, "boundary": boundary }).to_string());
        let url = self.config.routes.scopes();
        let timeout = self.config.read_timeout;
        let answer =
            self.read(RouteKind::CreateScope, Method::POST, url.clone(), Weight::Light, timeout, Some(body)).await?;
        decode(&url, &answer.body)
    }

    async fn export_scope(&self, scope: &ScopePath, at: Option<Seq>) -> Result<RawExport, WpsError> {
        let url = query(self.config.routes.scope(scope, "export"), &[("at", at.map(|seq| seq.to_string()))]);
        let timeout = self.config.export_timeout;
        let answer = self.read(RouteKind::Export, Method::GET, url.clone(), Weight::Heavy, timeout, None).await?;
        let base = answer.headers.get("x-base-version").and_then(|value| value.to_str().ok()?.parse::<u64>().ok());
        let base = base.ok_or_else(|| WpsError::Invalid { text: format!("{url} answered no x-base-version") })?;
        Ok(RawExport { base: Seq::new(base), bytes: answer.body })
    }

    async fn export_flagged(
        &self,
        scope: &ScopePath,
        flags: ExportFlags,
        at: &BTreeMap<ScopePath, Seq>,
    ) -> Result<Bytes, WpsError> {
        let mut url = self.config.routes.scope(scope, "export");
        for (flag, set) in flags.named() {
            url.query_pairs_mut().append_pair(flag, if set { "1" } else { "0" });
        }
        for (read, seq) in at {
            url.query_pairs_mut().append_pair("at", &format!("{read}@{seq}"));
        }
        let timeout = self.config.export_timeout;
        let answer = self.read(RouteKind::Export, Method::GET, url, Weight::Heavy, timeout, None).await?;
        Ok(answer.body)
    }

    async fn entity(&self, scope: &ScopePath, id: &EntityId, at: Option<Seq>) -> Result<WireEntity, WpsError> {
        let url = self.config.routes.scope_item(scope, "entities", &id.to_string());
        self.get(query(url, &[("at", at.map(|seq| seq.to_string()))]), Weight::Light).await
    }

    async fn trail(&self, scope: &ScopePath, id: &EntityId) -> Result<Option<WireTrail>, WpsError> {
        self.get_or_none(self.config.routes.scope_item(scope, "history", &id.to_string()), "entity_unknown").await
    }

    async fn entities(
        &self,
        scope: &ScopePath,
        listing: &EntityQuery,
    ) -> Result<(Vec<WireListed>, Option<String>), WpsError> {
        let url = query(
            self.config.routes.scope(scope, "entities"),
            &[
                ("at", listing.at.map(|seq| seq.to_string())),
                ("under", listing.under.as_ref().map(ToString::to_string)),
                ("provenance", listing.provenance.clone()),
                ("after", listing.after.clone()),
                ("limit", listing.limit.map(|limit| limit.to_string())),
            ],
        );
        let answer: Value = self.get(url, Weight::Light).await?;
        let page = serde_json::from_value(answer.get("entities").cloned().unwrap_or(Value::Null))
            .map_err(|fault| WpsError::Invalid { text: format!("an entity listing: {fault}") })?;
        Ok((page, answer.get("next").and_then(Value::as_str).map(str::to_owned)))
    }

    async fn referrers(
        &self,
        scope: &ScopePath,
        id: &EntityId,
        at: Option<Seq>,
        after: Option<String>,
    ) -> Result<(Vec<WireReferrer>, Option<String>), WpsError> {
        let url = self.config.routes.scope_item(scope, "referrers", &id.to_string());
        let url = query(url, &[("at", at.map(|seq| seq.to_string())), ("after", after)]);
        let answer: Value = self.get(url, Weight::Light).await?;
        let page = serde_json::from_value(answer.get("referrers").cloned().unwrap_or(Value::Null))
            .map_err(|fault| WpsError::Invalid { text: format!("a referrer listing: {fault}") })?;
        Ok((page, answer.get("next").and_then(Value::as_str).map(str::to_owned)))
    }

    async fn resolve(&self, scope: &ScopePath, reference: &str) -> Result<Resolution, WpsError> {
        let url = query(self.config.routes.scope(scope, "resolve"), &[("ref", Some(reference.to_owned()))]);
        let (weight, timeout) = (Weight::Light, self.config.read_timeout);
        let answer = self.read(RouteKind::Resolve, Method::GET, url.clone(), weight, timeout, None).await?;
        decode(&url, &answer.body)
    }

    async fn changesets(&self, scope: &ScopePath, before: Option<Seq>, limit: u16) -> Result<Vec<LogEntry>, WpsError> {
        let url = query(
            self.config.routes.scope(scope, "changesets"),
            &[("before", before.map(|seq| seq.to_string())), ("limit", Some(limit.to_string()))],
        );
        let answer: Value = self.get(url, Weight::Light).await?;
        serde_json::from_value(answer.get("changesets").cloned().unwrap_or(Value::Null))
            .map_err(|fault| WpsError::Invalid { text: format!("a changeset log: {fault}") })
    }

    async fn changeset_named(&self, scope: &ScopePath, operation: &str) -> Result<Option<LogEntry>, WpsError> {
        let url = query(self.config.routes.scope(scope, "changesets"), &[("operation", Some(operation.to_owned()))]);
        let answer: Value = self.get(url, Weight::Light).await?;
        let entries: Vec<LogEntry> =
            serde_json::from_value(answer.get("changesets").cloned().unwrap_or(Value::Null))
                .map_err(|fault| WpsError::Invalid { text: format!("a changeset log: {fault}") })?;
        Ok(entries.into_iter().find(|entry| entry.operation.as_deref() == Some(operation)))
    }

    async fn post_changeset(&self, changeset: &WireChangeset) -> Result<WireApplied, PostFailure> {
        let body = serde_json::to_vec(changeset)
            .map_err(|fault| PostFailure::Answered(WpsError::Invalid { text: format!("a changeset: {fault}") }))?;
        let sent = body.len() as u64;
        let url = self.config.routes.scope(&changeset.scope, "changesets");
        let request = self.request(Method::POST, url.clone(), self.config.write_timeout, Some(&Bytes::from(body)));
        let answer =
            self.once(Weight::Heavy, request).await.map_err(|fault| PostFailure::Lost { detail: fault.to_string() })?;
        // Only WPS's own refusal of the body, its refusal object in hand, says nothing was written; a 201 this client
        // cannot read, a 5xx and any answer without that object leave the outcome unknown (LEAD-DECISIONS entry 54).
        match answer.status {
            201 => decode(&url, &answer.body).map_err(|fault| PostFailure::Lost {
                detail: format!("201 from {url}, whose answer did not read, so whether it applied is unknown: {fault}"),
            }),
            500..=599 => Err(PostFailure::Lost {
                detail: format!("{} from {url}: {}", answer.status, refusal::sentence_of(&answer.body)),
            }),
            status if Refusal::of(&answer.body).is_none() => Err(PostFailure::Lost {
                detail: format!(
                    "{status} from {url} without world-proto-server's refusal: {}",
                    refusal::sentence_of(&answer.body)
                ),
            }),
            status => Err(match refusal::read(RouteKind::Other, status, &answer.body, sent) {
                Reading::Error(error) => PostFailure::Answered(error),
                Reading::EndedByPostgres { text } => PostFailure::EndedByPostgres { text },
                Reading::GeneratorsMoved { details, .. } => {
                    let stated = changeset.generators.as_deref().unwrap_or_default();
                    PostFailure::Answered(refusal::generators_moved(&details, stated))
                }
            }),
        }
    }

    async fn import(&self, scope: &ScopePath, base: Seq, document: &Value) -> Result<WireImport, WpsError> {
        let url = query(self.config.routes.scope(scope, "import"), &[("base", Some(base.to_string()))]);
        let body = Bytes::from(document.to_string());
        let timeout = self.config.write_timeout;
        let answer = self.read(RouteKind::Other, Method::POST, url.clone(), Weight::Heavy, timeout, Some(body)).await?;
        decode(&url, &answer.body)
    }

    async fn components(&self) -> Result<Vec<WireDefinition>, WpsError> {
        let answer: Value = self.get(self.config.routes.registry(), Weight::Light).await?;
        serde_json::from_value(answer.get("components").cloned().unwrap_or(Value::Null))
            .map_err(|fault| WpsError::Invalid { text: format!("GET /registry: {fault}") })
    }

    async fn component(&self, name: &str) -> Result<Option<WireComponent>, WpsError> {
        self.get_or_none(self.config.routes.registry_component(name), "component_undefined").await
    }

    async fn generators(&self) -> Result<WireGenerators, WpsError> {
        self.get(self.config.routes.generators(), Weight::Light).await
    }

    async fn generator(&self, name: &str) -> Result<WireGenerator, WpsError> {
        let answer: Value = self.get(self.config.routes.generator(name), Weight::Light).await?;
        let versions = answer.get("versions").cloned().unwrap_or(Value::Array(Vec::new()));
        let versions = serde_json::from_value(versions)
            .map_err(|fault| WpsError::Invalid { text: format!("generator {name}'s versions: {fault}") })?;
        Ok(WireGenerator { generator: name.to_owned(), versions })
    }

    async fn subscribe(&self, scope: &ScopePath) -> Result<EventStream, WpsError> {
        let url = self.config.routes.scope(scope, "events");
        self.sent.fetch_add(1, Ordering::Relaxed);
        let answer = self
            .http
            .get(url.clone())
            .header(ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(|fault| WpsError::Unavailable { attempts: 1, last: format!("{url}: {fault}") })?;
        let status = answer.status().as_u16();
        if status != 200 {
            let body = answer.bytes().await.unwrap_or_default();
            return Err(match refusal::read(RouteKind::Other, status, &body, 0) {
                Reading::Error(error) => error,
                Reading::EndedByPostgres { text } | Reading::GeneratorsMoved { text, .. } => {
                    WpsError::ServerFault { status, text }
                }
            });
        }
        let bytes = answer.bytes_stream();
        let events = futures_util::stream::unfold((bytes, String::new()), |(mut bytes, mut buffer)| async move {
            loop {
                if let Some(end) = buffer.find("\n\n") {
                    let block: String = buffer.drain(..end + 2).collect();
                    match server_sent(&block) {
                        Some(event) => return Some((event, (bytes, buffer))),
                        None => continue,
                    }
                }
                match bytes.next().await {
                    Some(Ok(chunk)) => buffer.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n")),
                    Some(Err(fault)) => {
                        let lost =
                            WpsError::Unavailable { attempts: 1, last: format!("the event stream broke: {fault}") };
                        return Some((Err(lost), (bytes, String::new())));
                    }
                    None => return None,
                }
            }
        });
        Ok(Box::pin(events))
    }
}

/// One server-sent event block: a `changeset` event's data, or `None` for a comment or another event.
fn server_sent(block: &str) -> Option<Result<WireEvent, WpsError>> {
    let (mut name, mut data) = (None, Vec::new());
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            name = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    if name != Some("changeset") || data.is_empty() {
        return None;
    }
    Some(
        serde_json::from_str(&data.join("\n"))
            .map_err(|fault| WpsError::Invalid { text: format!("an event: {fault}") }),
    )
}
