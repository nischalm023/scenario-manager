//! Within `runs`, `events` is the run loops' `RunEvents` over the journal (spec 08 section 3.6; spec 01 sections 3.12
//! and 3.17): each transition becomes one `run.status` entry and a round's last ending one `submission.ended`, each
//! under spec 08's dedup key and written in the site's live conversation, which is opened when none is, so a run's
//! events follow the site across a Dismiss and a repeat after a restart writes nothing. Progress is sent live only.

use desunits::{SimDuration, SimTime};
use run_invocation::{Fault, Round, RunEvents, RunningProgress, Transition};
use serde_json::json;
use sqlx::{Postgres, Row, Transaction};
use studio_protocol::{
    EndedBy, JournalKind, LiveItem, LiveKind, RunCount, RunPosition, RunProgress, RunStatus, SitePath, Subject,
    SubmissionEnded,
};

use crate::AppState;
use crate::conversations::open_site;
use crate::db::{decode, read_word, word};
use crate::journal::{Appended, JournalError, NewEntry, Written, append_to_site};

/// The run loops' events writer: the API's journal.
#[derive(Clone)]
pub struct JournalEvents {
    app: AppState,
}

impl JournalEvents {
    pub fn new(app: AppState) -> Self {
        Self { app }
    }
}

impl RunEvents for JournalEvents {
    async fn status_changed(&self, transition: &Transition) -> Result<(), Fault> {
        let mut tx = self.app.pool.begin().await?;
        let row = sqlx::query(
            "SELECT r.submission, r.position, r.duration_sim_ms, sc.name, sc.site_scope, s.submitted_by, \
                 (SELECT count(*) FROM runs o WHERE o.submission = r.submission) AS of, \
                 a.progress_sim_ms, a.ended_by, a.stop_reason, a.stop_detail \
             FROM runs r JOIN submissions s ON s.id = r.submission JOIN scenarios sc ON sc.id = r.scenario \
             JOIN attempts a ON a.run_id = r.id AND a.n = $2 WHERE r.id = $1",
        )
        .bind(transition.key.run.as_str())
        .bind(i32::try_from(transition.key.n.0).unwrap_or(i32::MAX))
        .fetch_one(&mut *tx)
        .await?;
        let ended = transition.to.ended();
        let text = |column: &str| -> Result<Option<String>, Fault> {
            Ok(if ended { row.try_get::<Option<String>, _>(column)? } else { None })
        };
        let body = RunStatus {
            run: transition.key.run.clone(),
            submission: row.try_get::<String, _>("submission")?.parse()?,
            position: RunPosition(u32::try_from(row.try_get::<i32, _>("position")?)?),
            of: RunCount(u32::try_from(row.try_get::<i64, _>("of")?)?),
            attempt: transition.key.n,
            state: transition.to,
            from: transition.from,
            sim_ms: SimTime::ms(u64::try_from(row.try_get::<i64, _>("progress_sim_ms")?)?),
            ended_by: text("ended_by")?.map(|by| read_word::<EndedBy>(&by)).transpose()?,
            stop_reason: text("stop_reason")?,
            stop_detail: text("stop_detail")?,
            scenario: Some(row.try_get("name")?),
            duration_sim_ms: Some(SimDuration::ms(u64::try_from(row.try_get::<i64, _>("duration_sim_ms")?)?)),
        };
        let key = format!("run.status:{}:{}:{}", transition.key.run, transition.key.n.0, word(transition.to));
        let mut entry = NewEntry::new(JournalKind::RunStatus, json!(body));
        entry.dedup_key = Some(key);
        let site: SitePath = row.try_get::<String, _>("site_scope")?.parse()?;
        let opener: Subject = row.try_get::<String, _>("submitted_by")?.parse()?;
        let written = journal_on_site(&self.app, &mut tx, &site, &opener, entry).await?;
        tx.commit().await?;
        self.app.hub.announce(written);
        Ok(())
    }

    async fn submission_ended(&self, round: &Round) -> Result<(), Fault> {
        let mut tx = self.app.pool.begin().await?;
        let (site, opener): (String, String) = sqlx::query_as(
            "SELECT sc.site_scope, s.submitted_by FROM submissions s \
             JOIN runs r ON r.submission = s.id JOIN scenarios sc ON sc.id = r.scenario WHERE s.id = $1 LIMIT 1",
        )
        .bind(round.submission.as_str())
        .fetch_one(&mut *tx)
        .await?;
        let rounds = i32::try_from(round.attempts.0).unwrap_or(i32::MAX);
        sqlx::query(
            "UPDATE submissions SET ended_rounds = array_append(ended_rounds, $2) \
             WHERE id = $1 AND NOT ($2 = ANY(ended_rounds))",
        )
        .bind(round.submission.as_str())
        .bind(rounds)
        .execute(&mut *tx)
        .await?;
        let counts = round.counts;
        let of = RunCount(counts.finished.0 + counts.cancelled.0 + counts.failed.0 + counts.partial.0);
        let body = SubmissionEnded { submission: round.submission.clone(), of, counts };
        let mut entry = NewEntry::new(JournalKind::SubmissionEnded, json!(body));
        entry.dedup_key = Some(format!("submission.ended:{}:{}", round.submission, round.attempts.0));
        let written = journal_on_site(&self.app, &mut tx, &site.parse()?, &opener.parse()?, entry).await?;
        tx.commit().await?;
        self.app.hub.announce(written);
        Ok(())
    }

    async fn progress(&self, progress: &RunningProgress) -> Result<(), Fault> {
        let live: Option<String> = sqlx::query_scalar(
            "SELECT c.id FROM runs r JOIN scenarios sc ON sc.id = r.scenario \
             JOIN conversations c ON c.site = sc.site_scope AND c.state = 'live' WHERE r.id = $1",
        )
        .bind(progress.key.run.as_str())
        .fetch_optional(&self.app.pool)
        .await?;
        let Some(conversation) = live else { return Ok(()) };
        let body = RunProgress {
            run: progress.key.run.clone(),
            attempt: progress.key.n,
            sim_ms: progress.sim_ms,
            values: progress.values.clone(),
        };
        let item = LiveItem { kind: LiveKind::RunProgress, body: json!(body) };
        self.app.hub.live(&conversation.parse().map_err(decode)?, item)?;
        Ok(())
    }
}

/// Append `entry` to `site`'s live conversation inside `tx`, opening one for `opener` when nobody has: its entry, for
/// the hub once `tx` commits, or `None` when its dedup key already holds an entry of the site.
pub async fn journal_on_site(
    app: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    site: &SitePath,
    opener: &Subject,
    entry: NewEntry,
) -> Result<Option<Written>, Fault> {
    for _ in 0..3 {
        match append_to_site(tx, site, entry.clone()).await {
            Ok(Appended::Wrote(written)) => return Ok(Some(written)),
            Ok(Appended::Held(_)) => return Ok(None),
            Err(JournalError::NoLiveConversation(_)) => {
                open_site(tx, site, opener, &app.model_label).await?;
            }
            Err(fault) => return Err(fault.into()),
        }
    }
    Err(format!("{site}'s conversation kept closing while a run's event was written").into())
}
