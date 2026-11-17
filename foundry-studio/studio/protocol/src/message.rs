//! Within `studio-protocol`, `message` is what a person tells the agent (spec 01 sections 3.5 and 3.6; spec 02
//! section 3.3, step 10): a message, with the body a page posts, the 202 that accepts it, and the `message.user` entry
//! the journal keeps, whose `client_message_id` lets the sending page settle its optimistic entry whichever of the two
//! arrives first. A panel's own act told to the agent is `panel_event`'s.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::grammar::Ulid;
use crate::journal::JournalSeq;
use crate::page::PageContext;
use crate::person::Person;

/// Where a message was written: the composer, or a panel act that answers the agent (spec 02 section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "message.ts")]
pub enum MessageSource {
    Composer,
    Panel,
}

/// `POST /api/conversations/{c}/messages`'s body. `context` is null when the person left the page context out;
/// `act` names the panel act that sent a `panel` message ("Choose template"), so its entry can read "Sent from the
/// Template panel by Choose template" (spec 02 section 3.1, UserEntry), and is absent from the composer's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "message.ts")]
pub struct PostMessage {
    pub text: String,
    pub context: Option<PageContext>,
    pub source: MessageSource,
    pub browser_tab: String,
    pub client_message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub act: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unattended: Option<bool>,
}

/// The 202 a message is accepted with: its turn, its entry's number, and the id it was sent with; `answered` or
/// `denied` when it answered the question or denied the approval its turn waited on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "message.ts")]
pub struct MessageAccepted {
    pub turn: Ulid,
    pub seq: JournalSeq,
    pub client_message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub answered: Option<Ulid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub denied: Option<Ulid>,
}

/// `message.user`'s body: the message as the person sent it, with who sent it and from which browser tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "message.ts")]
pub struct UserMessage {
    pub author: Person,
    pub text: String,
    pub context: Option<PageContext>,
    pub source: MessageSource,
    pub browser_tab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub client_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub act: Option<String>,
}
