//! Within `studio-api`, `identity` is who is asking (spec 01 section 3.2). The login gate checks the session and
//! forwards the person in four headers; the identity layer reads them on every request to the gate-facing listener
//! but `GET /healthz`, upserts the person's `people` row and hands the request on with an `Identity`, and a request
//! that names nobody is refused with `identity-missing` before any route runs. Locally, `STUDIO_DEV_IDENTITY` stands
//! in for the gate. Identity never comes from a body or a tool input. `GET /api/me` answers whom the service took
//! the asker to be.

use std::str::{self, FromStr};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use rustc_hash::FxHashSet;
use sqlx::{Error as SqlxError, PgPool};
use studio_protocol::{Me, Problem, ProblemType, Subject};

use crate::AppState;
use crate::problem::{ProblemAnswer, internal};

/// The four headers the gate forwards a person in, in `STUDIO_IDENTITY_HEADERS`'s order: the stable subject, the
/// email, the display name and the Keycloak roles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityHeaders {
    pub subject: HeaderName,
    pub email: HeaderName,
    pub name: HeaderName,
    pub roles: HeaderName,
}

/// Spec 01 section 3.1's default names, oauth2-proxy's; unconfirmed until the gate, which is Nithin's, is set up.
impl Default for IdentityHeaders {
    fn default() -> Self {
        Self {
            subject: HeaderName::from_static("x-auth-request-user"),
            email: HeaderName::from_static("x-auth-request-email"),
            name: HeaderName::from_static("x-auth-request-preferred-username"),
            roles: HeaderName::from_static("x-auth-request-groups"),
        }
    }
}

impl FromStr for IdentityHeaders {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let expected = || "four header names joined by `,`: subject, email, name, roles".to_owned();
        let names = text.split(',').map(|name| HeaderName::from_str(name.trim()).map_err(|_| expected()));
        match names.collect::<Result<Vec<_>, _>>()?.as_slice() {
            [subject, email, name, roles] => {
                Ok(Self { subject: subject.clone(), email: email.clone(), name: name.clone(), roles: roles.clone() })
            }
            _ => Err(expected()),
        }
    }
}

/// The person every request is taken to be when `STUDIO_DEV_IDENTITY` (`subject,email,name`) is set, which only
/// `STUDIO_ENV=local` allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevIdentity {
    pub subject: Subject,
    pub email: String,
    pub name: String,
}

impl FromStr for DevIdentity {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = text.splitn(3, ',').map(str::trim).collect();
        match parts.as_slice() {
            [subject, email, name] if !email.is_empty() && !name.is_empty() => Ok(Self {
                subject: subject.parse().map_err(|_| "a subject that is not blank".to_owned())?,
                email: (*email).to_owned(),
                name: (*name).to_owned(),
            }),
            _ => Err("subject,email,name, none of them blank".to_owned()),
        }
    }
}

/// How the service tells who is asking: from the gate's headers, or locally from `STUDIO_DEV_IDENTITY`; and who
/// among them is a developer.
#[derive(Debug, Clone)]
pub struct IdentityRules {
    pub headers: IdentityHeaders,
    /// The Keycloak role that makes a person a developer (`STUDIO_DEVELOPER_ROLE`).
    pub developer_role: String,
    /// Subjects that are developers whatever their roles; empty unless `STUDIO_ENV` is local.
    pub developers: FxHashSet<Subject>,
    pub dev_identity: Option<DevIdentity>,
}

/// Who is asking: spec 01 section 3.2's `Identity`, put in a request's extensions by `resolve`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub subject: Subject,
    pub email: String,
    pub name: String,
    pub developer: bool,
}

/// Why a request names nobody: a header it lacks, holds blank or in bytes that are not text, or holds twice, which
/// would let a client's copy stand beside the gate's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingIdentity {
    pub header: HeaderName,
    pub why: &'static str,
}

impl IdentityRules {
    /// Who `headers` name. Any of the subject, email and name headers missing or blank names nobody; the roles
    /// header may be absent or blank, a person with no roles. Each header is read only when it appears once.
    pub fn identify(&self, headers: &HeaderMap) -> Result<Identity, MissingIdentity> {
        if let Some(dev) = &self.dev_identity {
            let developer = self.developers.contains(&dev.subject);
            return Ok(Identity {
                subject: dev.subject.clone(),
                email: dev.email.clone(),
                name: dev.name.clone(),
                developer,
            });
        }
        let blank = || MissingIdentity { header: self.headers.subject.clone(), why: "is blank" };
        let subject: Subject = text(headers, &self.headers.subject)?.parse().map_err(|_| blank())?;
        let email = text(headers, &self.headers.email)?;
        let name = text(headers, &self.headers.name)?;
        let roles = once(headers, &self.headers.roles)?.unwrap_or_default();
        let has_role = roles.split(',').any(|role| role.trim() == self.developer_role);
        Ok(Identity { developer: has_role || self.developers.contains(&subject), subject, email, name })
    }
}

/// One identity header's text, trimmed and never blank.
fn text(headers: &HeaderMap, header: &HeaderName) -> Result<String, MissingIdentity> {
    let missing = |why| MissingIdentity { header: header.clone(), why };
    let text = once(headers, header)?.ok_or(missing("is absent"))?;
    if text.is_empty() { Err(missing("is blank")) } else { Ok(text) }
}

/// A header's trimmed text when it appears, refused when it appears twice or holds bytes that are not UTF-8.
fn once(headers: &HeaderMap, header: &HeaderName) -> Result<Option<String>, MissingIdentity> {
    let missing = |why| MissingIdentity { header: header.clone(), why };
    let mut values = headers.get_all(header).iter();
    let Some(value) = values.next() else { return Ok(None) };
    if values.next().is_some() {
        return Err(missing("appears more than once"));
    }
    let text = str::from_utf8(value.as_bytes()).map_err(|_| missing("does not hold UTF-8 text"))?;
    Ok(Some(text.trim().to_owned()))
}

/// The identity layer: read who is asking, record them in `people`, and hand the request on with its `Identity`; a
/// request that names nobody is answered `identity-missing` and goes no further.
pub async fn resolve(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let identity = match state.identity.identify(request.headers()) {
        Ok(identity) => identity,
        Err(MissingIdentity { header, why }) => {
            let detail = format!("The request names nobody: its {header} header {why}. Sign in through the gate.");
            return ProblemAnswer::from(Problem::new(ProblemType::IdentityMissing, detail).at(path)).into_response();
        }
    };
    if let Err(fault) = record(&state.pool, &identity).await {
        return internal("record who is asking", fault, &path).into_response();
    }
    request.extensions_mut().insert(identity);
    next.run(request).await
}

/// Upsert the person's row: first seen once, last seen now (wall clock), email and name as the gate says them today.
async fn record(pool: &PgPool, identity: &Identity) -> Result<(), SqlxError> {
    sqlx::query(
        "INSERT INTO people (subject, email, name) VALUES ($1, $2, $3) \
         ON CONFLICT (subject) DO UPDATE SET email = EXCLUDED.email, name = EXCLUDED.name, last_seen_at = now()",
    )
    .bind(identity.subject.as_str())
    .bind(&identity.email)
    .bind(&identity.name)
    .execute(pool)
    .await?;
    Ok(())
}

/// `GET /api/me`: whom the service took the asker to be, the runner image they chose, and the agent's provider and
/// model (spec 01 section 3.6).
pub async fn me(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
) -> Result<Json<Me>, ProblemAnswer> {
    let runner_image = sqlx::query_scalar::<_, Option<String>>("SELECT runner_image FROM people WHERE subject = $1")
        .bind(identity.subject.as_str())
        .fetch_one(&state.pool)
        .await
        .map_err(|fault| internal("read the person's runner image", fault, uri.path()))?;
    let Identity { subject, email, name, developer } = identity;
    Ok(Json(Me { subject, email, name, developer, runner_image, agent: state.agent.model() }))
}
