//! Within `agent`, the `transcript` is what the model reads of a conversation (spec 01 sections 3.4, 3.5 and 3.7):
//! `model_messages` in order, rig's messages exactly as they were sent or received, the provider's items (Gemini's
//! thought signatures) included, appended only; a compaction's summary replacing everything before it; and the parts
//! of a turn's first user message: closing results for calls a stopped or interrupted turn left unanswered, the
//! context notes the model has not seen, the page context and project, then the person's words. The words never go
//! inside a tool result.

use agent_loop::rig::{Message, Text, ToolCall, ToolResult, ToolResultContent, UserContent};
use serde_json::{Value, json};
use sqlx::{Error as SqlxError, PgConnection, Postgres, Transaction};
use studio_protocol::{JournalKind, PageContext, Ulid};

use crate::db::decode;

/// One row of `model_messages`.
pub(super) struct Row {
    pub n: i32,
    pub message: Message,
    pub summary_of: Option<i32>,
}

/// Every row of the conversation's transcript, in order.
pub(super) async fn rows(conn: &mut PgConnection, conversation: &Ulid) -> Result<Vec<Row>, SqlxError> {
    let found: Vec<(i32, Value, Option<i32>)> =
        sqlx::query_as("SELECT n, content, summary_of FROM model_messages WHERE conversation = $1 ORDER BY n")
            .bind(conversation.as_str())
            .fetch_all(conn)
            .await?;
    (found.into_iter())
        .map(|(n, content, summary_of)| {
            Ok(Row { n, message: serde_json::from_value(content).map_err(decode)?, summary_of })
        })
        .collect()
}

/// What a request sends: the rows from the newest summary on, the summary's words the first block of the first user
/// message after it, and nothing before it (spec 01 section 3.4, "Compaction").
pub(super) fn history(rows: &[Row]) -> Vec<Message> {
    let from = rows.iter().rposition(|row| row.summary_of.is_some()).unwrap_or(0);
    let mut history: Vec<Message> = rows[from..].iter().map(|row| row.message.clone()).collect();
    if rows.get(from).is_some_and(|row| row.summary_of.is_some()) && history.len() > 1 {
        let summary = history.remove(0);
        if let (Message::User { content: summary }, Some(Message::User { content })) = (summary, history.first_mut()) {
            let mut joined = summary;
            joined.append(content);
            *content = joined;
        }
    }
    history
}

/// The calls of the transcript's last message when it is an assistant message no result answered: what a stopped or
/// interrupted turn left.
pub(super) fn unanswered(rows: &[Row]) -> Vec<ToolCall> {
    match rows.last().map(|row| &row.message) {
        Some(Message::Assistant(turn)) => turn.tool_calls().cloned().collect(),
        _ => Vec::new(),
    }
}

/// The result closing a call its turn never answered (spec 01 section 3.4, "Closing unanswered calls").
pub(super) fn closing(call: &ToolCall) -> ToolResult {
    let words = "This call was stopped or interrupted before it finished, so its effect is unknown; check before \
                 repeating it.";
    ToolResult { is_error: true, ..call.result(vec![ToolResultContent::Json { value: json!({ "stopped": words }) }]) }
}

/// Append `message` to the transcript as its next row, `summary_of` set for a compaction's summary.
pub(super) async fn append(
    tx: &mut Transaction<'_, Postgres>,
    conversation: &Ulid,
    turn: &Ulid,
    message: &Message,
    summary_of: Option<i32>,
) -> Result<i32, SqlxError> {
    let role = match message {
        Message::Assistant(_) => "assistant",
        _ => "user",
    };
    sqlx::query_scalar(
        "INSERT INTO model_messages (conversation, n, role, content, turn, summary_of) \
         SELECT $1, COALESCE(MAX(n), 0) + 1, $2, $3, $4, $5 FROM model_messages WHERE conversation = $1 RETURNING n",
    )
    .bind(conversation.as_str())
    .bind(role)
    .bind(json!(message))
    .bind(turn.as_str())
    .bind(summary_of)
    .fetch_one(&mut **tx)
    .await
}

/// A user message: the results first, in the calls' order, then each text block.
pub(super) fn user_message(results: Vec<ToolResult>, texts: Vec<String>) -> Message {
    let mut content: Vec<UserContent> = results.into_iter().map(UserContent::ToolResult).collect();
    content.extend(
        texts.into_iter().filter(|text| !text.trim().is_empty()).map(|text| UserContent::Text(Text::new(text))),
    );
    Message::User { content }
}

/// The context notes of a turn (spec 01 section 3.5, "What the model learns between turns"): every entry no turn
/// wrote numbered after `after` and before `before` whose kind tells the model what changed beside it, one sentence
/// each.
pub(super) async fn notes(
    conn: &mut PgConnection,
    conversation: &Ulid,
    after: i64,
    before: i64,
) -> Result<Option<String>, SqlxError> {
    let kinds = [
        JournalKind::PanelEvent,
        JournalKind::RunStatus,
        JournalKind::RunFinding,
        JournalKind::DraftApplied,
        JournalKind::DraftDiscarded,
        JournalKind::DraftStale,
    ]
    .map(JournalKind::as_str);
    let found: Vec<(String, Value)> = sqlx::query_as(
        "SELECT kind, body FROM journal WHERE conversation = $1 AND turn IS NULL AND seq > $2 AND seq < $3 \
         AND kind = ANY($4) ORDER BY seq",
    )
    .bind(conversation.as_str())
    .bind(after)
    .bind(before)
    .bind(&kinds[..])
    .fetch_all(conn)
    .await?;
    let text = |body: &Value, key: &str| body.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    let lines: Vec<String> = (found.iter())
        .map(|(kind, body)| match kind.as_str() {
            "panel.event" => text(body, "text"),
            "run.status" => format!("Run {} is now {}.", text(body, "run"), text(body, "state")),
            "run.finding" => format!("Run {} recorded a finding: {}", text(body, "run"), body["finding"]),
            "draft.applied" => format!("The draft changeset {} was applied: {}.", text(body, "draft"), body["seqs"]),
            "draft.discarded" => format!("The draft changeset {} was discarded.", text(body, "draft")),
            _ => format!("The draft changeset {} went out of date ({}).", text(body, "draft"), text(body, "reason")),
        })
        .filter(|line| !line.trim().is_empty())
        .map(|line| format!("- {line}"))
        .collect();
    Ok((!lines.is_empty()).then(|| format!("Context notes since your last turn:\n{}", lines.join("\n"))))
}

/// The page context as one line (spec 03 section 3.4): what the person's tab showed when they sent the message.
pub(super) fn page_line(context: &PageContext) -> String {
    let mut line = format!("Page context: the person's tab showed \"{}\" at {}", context.title, context.route);
    if let Some(selection) = &context.selection {
        line.push_str(&format!(", with {} ({}) selected", selection.label, selection.key));
    }
    if !context.seen.is_empty() {
        let seen: Vec<String> =
            context.seen.iter().map(|seen| format!("{} at changeset {}", seen.scope, seen.seq)).collect();
        line.push_str(&format!("; it read {}", seen.join(", ")));
    }
    line.push('.');
    line
}
