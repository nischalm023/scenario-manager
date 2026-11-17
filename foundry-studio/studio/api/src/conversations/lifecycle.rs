//! Within `conversations`, `lifecycle` is how a conversation comes to be, binds to its site and ends (spec 01 section
//! 3.17, ruling D94). Opening answers a site's live conversation, or the person's new-site thread, making it when
//! there is none; a site's next conversation numbers on from where its last one stopped. Create site binds a thread
//! to the site it made inside its own transaction. Dismiss archives a site's conversation and opens the next in one
//! transaction, or discards a new-site thread. Each change journals `conversation.changed`, which is an archived or
//! discarded conversation's last entry.

use serde_json::Value;
use sqlx::{Error as SqlxError, PgConnection, Postgres, Transaction};
use studio_protocol::{
    Conversation, ConversationChanged, ConversationState, Dismissed, JournalKind, Person, Problem, ProblemType,
    SitePath, Subject, Ulid,
};
use thiserror::Error;

use super::rows::{live_of_site, live_thread_of, load};
use crate::AppState;
use crate::db::{decode, mint_ulid};
use crate::identity::Identity;
use crate::journal::{Appended, JournalError, NewEntry, Written, append, open_turn};
use crate::problem::{ProblemAnswer, internal};
use crate::projects::default_project;

/// How many times opening looks again after another request made the conversation first.
const OPEN_ATTEMPTS: usize = 3;
const ONE_LIVE_PER_SITE: &str = "conversations_one_live_per_site";

#[derive(Debug, Error)]
pub enum ConversationError {
    #[error("there is no conversation {0}")]
    NotFound(Ulid),
    #[error("this new site's conversation is {owner}'s")]
    NotOwner { owner: Subject },
    #[error("a turn of the conversation runs or waits")]
    TurnRunning,
    #[error("no project is the default, so a new conversation has none to belong to")]
    ProjectRequired,
    #[error("the conversation {0} is no live new-site thread, so nothing binds it")]
    NotAThread(Ulid),
    #[error("the site {0} already has a conversation, so no thread binds to it")]
    SiteHasConversation(SitePath),
    #[error("{0}")]
    Unexpected(String),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Database(#[from] SqlxError),
}

impl ConversationError {
    /// This error as a route answers it for the request at `instance`.
    pub fn answer(&self, instance: &str) -> ProblemAnswer {
        let problem = match self {
            Self::NotFound(conversation) => {
                Problem::new(ProblemType::NotFound, format!("There is no conversation {conversation}."))
            }
            Self::NotOwner { owner } => Problem::new(
                ProblemType::NotConversationOwner,
                format!("This new site's conversation is {owner}'s; you can read it."),
            ),
            Self::TurnRunning => Problem::new(ProblemType::TurnRunning, "The agent is working. Stop it or wait."),
            Self::ProjectRequired => Problem::new(
                ProblemType::ProjectRequired,
                "No project is the default, so a new conversation has none to belong to; make one first.",
            ),
            Self::Journal(journal) => return journal.answer(instance),
            other => return internal("change the conversation", other, instance),
        };
        problem.at(instance).into()
    }
}

/// The live conversation of `site`, made by `by` when the site has none; true when this call made it.
pub async fn open_site(
    conn: &mut PgConnection,
    site: &SitePath,
    by: &Subject,
    model_label: &str,
) -> Result<(Conversation, bool), ConversationError> {
    for _ in 0..OPEN_ATTEMPTS {
        if let Some(live) = live_of_site(&mut *conn, site, model_label).await? {
            return Ok((live, false));
        }
        let project = default_project(&mut *conn).await?.ok_or(ConversationError::ProjectRequired)?;
        let id = mint_ulid();
        let made = sqlx::query(
            "INSERT INTO conversations (id, project, site, created_by, first_seq, last_seq) \
             SELECT $1, $2, $3, $4, last + 1, last \
             FROM (SELECT COALESCE(MAX(last_seq), 0) AS last FROM conversations WHERE site = $3) AS numbered \
             ON CONFLICT (site) WHERE state = 'live' DO NOTHING",
        )
        .bind(id.as_str())
        .bind(project.as_str())
        .bind(site.as_str())
        .bind(by.as_str())
        .execute(&mut *conn)
        .await?;
        if made.rows_affected() == 1 {
            return Ok((load_existing(conn, &id, model_label).await?, true));
        }
    }
    Err(ConversationError::Unexpected(format!("{site}'s live conversation kept changing while it was opened")))
}

/// The live new-site thread of `by`, made when they have none; true when this call made it.
pub async fn open_thread(
    conn: &mut PgConnection,
    by: &Subject,
    model_label: &str,
) -> Result<(Conversation, bool), ConversationError> {
    for _ in 0..OPEN_ATTEMPTS {
        if let Some(live) = live_thread_of(&mut *conn, by, model_label).await? {
            return Ok((live, false));
        }
        let project = default_project(&mut *conn).await?.ok_or(ConversationError::ProjectRequired)?;
        let id = mint_ulid();
        let made = sqlx::query(
            "INSERT INTO conversations (id, project, created_by) VALUES ($1, $2, $3) \
             ON CONFLICT (created_by) WHERE site IS NULL AND state = 'live' DO NOTHING",
        )
        .bind(id.as_str())
        .bind(project.as_str())
        .bind(by.as_str())
        .execute(&mut *conn)
        .await?;
        if made.rows_affected() == 1 {
            return Ok((load_existing(conn, &id, model_label).await?, true));
        }
    }
    Err(ConversationError::Unexpected(format!("{by}'s new-site thread kept changing while it was opened")))
}

async fn load_existing(
    conn: &mut PgConnection,
    id: &Ulid,
    model_label: &str,
) -> Result<Conversation, ConversationError> {
    load(conn, id, model_label).await?.ok_or_else(|| ConversationError::NotFound(id.clone()))
}

/// Bind the new-site thread `thread` to the site Create site made, inside Create site's own transaction (spec 05's
/// made-from record's): the thread takes the site and its name, keeps its numbers, and journals
/// `conversation.changed`, which the caller announces once that transaction commits. A site that already has a
/// conversation refuses the binding, since one number would then name two entries of the site.
pub async fn bind_thread(
    tx: &mut Transaction<'_, Postgres>,
    thread: &Ulid,
    site: &SitePath,
    title: &str,
    by: &Person,
) -> Result<Written, ConversationError> {
    let row: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT site, state FROM conversations WHERE id = $1 FOR UPDATE")
            .bind(thread.as_str())
            .fetch_optional(&mut **tx)
            .await?;
    match row {
        None => return Err(ConversationError::NotFound(thread.clone())),
        Some((None, state)) if state == ConversationState::Live.as_str() => {}
        Some(_) => return Err(ConversationError::NotAThread(thread.clone())),
    }
    let held: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM conversations WHERE site = $1)")
        .bind(site.as_str())
        .fetch_one(&mut **tx)
        .await?;
    if held {
        return Err(ConversationError::SiteHasConversation(site.clone()));
    }
    let bound = sqlx::query("UPDATE conversations SET site = $2, title = $3, bound_at = now() WHERE id = $1")
        .bind(thread.as_str())
        .bind(site.as_str())
        .bind(title)
        .execute(&mut **tx)
        .await;
    // A site route opened the site's conversation after the check, and the index refuses the second live one.
    if let Err(fault) = &bound
        && fault.as_database_error().and_then(|fault| fault.constraint()) == Some(ONE_LIVE_PER_SITE)
    {
        return Err(ConversationError::SiteHasConversation(site.clone()));
    }
    bound?;
    let changed = ConversationChanged {
        state: ConversationState::Live,
        site: Some(site.clone()),
        title: Some(title.to_owned()),
        next: None,
        by: by.clone(),
    };
    journal_changed(tx, thread, changed).await
}

/// Dismiss `conversation` for `by` (spec 01 section 3.17): a site's conversation is archived and the site's next live
/// one made, numbering on from its last entry; a new-site thread, which only its maker may dismiss, is discarded.
/// Refused while a turn of it runs or waits. A repeat answers the first outcome.
pub async fn dismiss(state: &AppState, conversation: &Ulid, by: &Identity) -> Result<Dismissed, ConversationError> {
    let label = &state.model_label;
    let mut tx = state.pool.begin().await?;
    let row: Option<(Option<String>, String, String, Option<String>)> =
        sqlx::query_as("SELECT site, state, created_by, title FROM conversations WHERE id = $1 FOR UPDATE")
            .bind(conversation.as_str())
            .fetch_optional(&mut *tx)
            .await?;
    let Some((site, current, owner, title)) = row else {
        return Err(ConversationError::NotFound(conversation.clone()));
    };
    let site: Option<SitePath> = site.map(|site| site.parse()).transpose().map_err(decode)?;
    match current.parse::<ConversationState>().map_err(decode)? {
        ConversationState::Archived => return ended_before(&mut tx, conversation, label).await,
        ConversationState::Discarded => {
            let discarded = Box::new(load_existing(&mut tx, conversation, label).await?);
            return Ok(Dismissed::Discarded { discarded });
        }
        ConversationState::Live => {}
    }
    if site.is_none() && owner != by.subject.as_str() {
        return Err(ConversationError::NotOwner { owner: owner.parse().map_err(decode)? });
    }
    if open_turn(&mut tx, conversation).await?.is_some() {
        return Err(ConversationError::TurnRunning);
    }
    let by_person = Person { subject: by.subject.clone(), name: by.name.clone() };
    let next = site.as_ref().map(|_| mint_ulid());
    let ended = if next.is_some() { ConversationState::Archived } else { ConversationState::Discarded };
    let changed = ConversationChanged { state: ended, site, title, next: next.clone(), by: by_person };
    let written = journal_changed(&mut tx, conversation, changed).await?;
    sqlx::query("UPDATE conversations SET state = $2, ended_at = now(), ended_by = $3 WHERE id = $1")
        .bind(conversation.as_str())
        .bind(ended.as_str())
        .bind(by.subject.as_str())
        .execute(&mut *tx)
        .await?;
    // TODO(drafts): discard the conversation's agent drafts not applied, or a thread's template draft, once
    // `changeset_drafts` (spec 01 section 3.11) and spec 05's template drafts exist.
    if let Some(next) = &next {
        sqlx::query(
            "INSERT INTO conversations (id, project, site, title, created_by, first_seq, last_seq) \
             SELECT $2, project, site, title, $3, last_seq + 1, last_seq FROM conversations WHERE id = $1",
        )
        .bind(conversation.as_str())
        .bind(next.as_str())
        .bind(by.subject.as_str())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    state.hub.announce([written]);
    let mut conn = state.pool.acquire().await?;
    let ended = Box::new(load_existing(&mut conn, conversation, label).await?);
    Ok(match next {
        Some(next) => {
            Dismissed::Archived { archived: ended, live: Box::new(load_existing(&mut conn, &next, label).await?) }
        }
        None => Dismissed::Discarded { discarded: ended },
    })
}

/// The first outcome of a Dismiss that archived `conversation`: it, and the conversation its last entry named next.
async fn ended_before(
    conn: &mut PgConnection,
    conversation: &Ulid,
    model_label: &str,
) -> Result<Dismissed, ConversationError> {
    let changed: Option<Value> = sqlx::query_scalar(
        "SELECT body FROM journal WHERE conversation = $1 AND kind = 'conversation.changed' ORDER BY seq DESC LIMIT 1",
    )
    .bind(conversation.as_str())
    .fetch_optional(&mut *conn)
    .await?;
    let next =
        changed.and_then(|body| serde_json::from_value::<ConversationChanged>(body).ok()).and_then(|body| body.next);
    let Some(next) = next else {
        return Err(ConversationError::Unexpected(format!("the archived conversation {conversation} names no next")));
    };
    let archived = Box::new(load_existing(conn, conversation, model_label).await?);
    Ok(Dismissed::Archived { archived, live: Box::new(load_existing(conn, &next, model_label).await?) })
}

async fn journal_changed(
    tx: &mut Transaction<'_, Postgres>,
    conversation: &Ulid,
    changed: ConversationChanged,
) -> Result<Written, ConversationError> {
    let body = serde_json::to_value(changed).expect("a conversation.changed body is strings and writes as JSON");
    match append(tx, conversation, NewEntry::new(JournalKind::ConversationChanged, body)).await? {
        Appended::Wrote(written) => Ok(written),
        Appended::Held(seq) => {
            Err(ConversationError::Unexpected(format!("an entry without a key was held by {seq:?}")))
        }
    }
}
