//! Within `studio-api`, `tools` are the agent's tools (spec 01 sections 3.8 and 3.9; brief I1.03 section 3): each
//! a direct typed call into the same service functions a person's routes call (D50's shape, back under D122), its
//! input checked against its full schema by the registry before it runs, its identity, project and conversation the
//! turn's and never the model's. Wired today: the five template tools and Create site (`templates`), the site list,
//! a site read and the map's find (`sites`), a run's metrics and one metric's series (`runs`), the four panel
//! commands (`panels`) and `ask_user` (`ask`); every tool
//! whose task is not built answers `not-built` naming it, and `site.tag` answers `wps-not-ready` (`stubs`).

mod ask;
mod panels;
mod runs;
mod sites;
mod stubs;
mod templates;

pub use panels::Shown;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use agent_loop::{Registry, Tool};
use axum::http::Uri;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::Error as SqlxError;
use studio_protocol::{CallId, JournalKind, PageContext, Problem, ProblemType, SitePath, ToolProgress, Ulid};
use tokio_util::sync::CancellationToken;
use wps_client::wps::Wps;

use crate::GateState;
use crate::agent::journal;
use crate::identity::Identity;
use crate::problem::ProblemAnswer;
use ask::AskUser;

/// The least wall-clock time between two progress notes of one call (spec 01 section 3.5's `tool.progress`).
const PROGRESS_EVERY_SECS: u64 = 2;

/// What a tool knows of the call it runs (spec 01 section 3.8's `ToolContext`): the service, who acts (the turn's
/// sender, or the approver when an approved input executes), the conversation and its site, the turn and the call,
/// the key its writes carry (the call's id, or the approval's), the call's cancellation, the page context of the
/// turn's message, and the location of the turn's last `panel.show`.
pub struct ToolContext<S: Wps> {
    pub gate: GateState<S>,
    pub identity: Identity,
    pub conversation: Ulid,
    pub site: Option<SitePath>,
    pub turn: Ulid,
    /// The turn's number, which a knob the agent sets records.
    pub turn_n: u32,
    pub call: CallId,
    pub key: String,
    pub cancel: CancellationToken,
    pub page: Option<PageContext>,
    pub shown: Arc<Mutex<Option<Shown>>>,
    pub approval: Option<Ulid>,
    pub progress: Arc<Mutex<Option<Instant>>>,
}

impl<S: Wps> ToolContext<S> {
    /// Tell the tabs how the call is going, at most once every 2 s (wall clock); a note sooner is dropped.
    pub fn progress(&self, note: &str) {
        {
            let mut last = self.progress.lock().unwrap_or_else(PoisonError::into_inner);
            if last.is_some_and(|at| at.elapsed().as_secs() < PROGRESS_EVERY_SECS) {
                return;
            }
            *last = Some(Instant::now());
        }
        let (app, conversation, turn) = (self.gate.app.clone(), self.conversation.clone(), self.turn.clone());
        let body = ToolProgress { call: self.call.clone(), note: note.to_owned(), fraction: None };
        tokio::spawn(async move {
            let written = async {
                let mut tx = app.pool.begin().await?;
                let caused = Some(body.call.to_string());
                let entry =
                    journal(&mut tx, &conversation, Some(&turn), JournalKind::ToolProgress, body, caused).await?;
                tx.commit().await?;
                Ok::<_, SqlxError>(entry)
            };
            match written.await {
                Ok(entry) => app.hub.announce([entry]),
                Err(fault) => eprintln!("studio-api: a progress note of turn {turn} was not journaled: {fault}"),
            }
        });
    }
}

/// Every tool of the catalogue, in the order the model reads them. A catalogue that does not register refuses here,
/// which the API's tests make sure never happens.
pub fn registry<S: Wps>() -> Registry<ToolContext<S>> {
    let mut tools: Vec<Box<dyn Tool<ToolContext<S>>>> = Vec::new();
    tools.extend(templates::tools::<S>());
    tools.extend(sites::tools::<S>());
    tools.extend(runs::tools::<S>());
    tools.extend(panels::tools::<S>());
    tools.push(Box::new(AskUser));
    tools.extend(stubs::tools::<S>());
    Registry::new(tools).expect("the catalogue registers: names legal and distinct, schemas compiling")
}

/// A tool's input read as the type its schema describes; the registry has checked it, so a refusal is a fault of the
/// schema's.
pub(crate) fn input<T: DeserializeOwned>(input: &Value) -> Result<T, ProblemAnswer> {
    serde_json::from_value(input.clone()).map_err(|fault| {
        Problem::new(ProblemType::ValidationFailed, format!("The input does not read: {fault}")).into()
    })
}

/// The address of the route a tool calls, or a refusal naming what does not read as one.
pub(crate) fn uri(path: &str) -> Result<Uri, ProblemAnswer> {
    path.parse().map_err(|_| Problem::new(ProblemType::ValidationFailed, format!("`{path}` is no address")).into())
}

/// `text` as a query value: every byte but the unreserved ones percent-encoded, so a value holding `&`, `#` or a
/// space reaches the route whole.
pub(crate) fn encoded(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// A route's refusal as the tool's: the same problem the person's route answers.
pub(crate) fn refused(answer: ProblemAnswer) -> Problem {
    *answer.0
}

impl From<ProblemAnswer> for Problem {
    fn from(answer: ProblemAnswer) -> Self {
        refused(answer)
    }
}
