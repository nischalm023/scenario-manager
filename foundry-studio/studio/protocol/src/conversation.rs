//! Within `studio-protocol`, `conversation` is a site's conversation as the API answers it (spec 01 sections 3.6 and
//! 3.17, ruling D94): one live conversation per site, and before a site exists one new-site thread per person.

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::clock::WallClock;
use crate::grammar::{SitePath, Ulid};
use crate::journal::JournalSeq;
use crate::person::{Person, Subject};

/// Where a conversation stands: `live` takes entries; Dismiss leaves a site's conversation `archived` and a new-site
/// thread `discarded`, and either then reads as history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "conversation.ts")]
pub enum ConversationState {
    Live,
    Archived,
    Discarded,
}

impl ConversationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Archived => "archived",
            Self::Discarded => "discarded",
        }
    }
}

impl FromStr for ConversationState {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "live" => Ok(Self::Live),
            "archived" => Ok(Self::Archived),
            "discarded" => Ok(Self::Discarded),
            _ => Err(format!("`{text}` is not live, archived or discarded")),
        }
    }
}

impl Display for ConversationState {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(self.as_str())
    }
}

/// What a waiting turn waits on, as a conversation's summary and a turn's projection say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "conversation.ts")]
pub enum WaitingOn {
    Question,
    Approval,
}

/// A conversation as the API answers it (spec 01 section 3.3's row): `site` is null for a new-site thread until
/// Create site binds it, and `first_seq` to `last_seq` are the numbers its journal holds in its site's one sequence
/// (`last_seq` is `first_seq - 1` while it holds none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "conversation.ts")]
pub struct Conversation {
    pub id: Ulid,
    pub project: Ulid,
    pub site: Option<SitePath>,
    pub state: ConversationState,
    /// The site's name once a new-site thread binds; null for a conversation a site route opened.
    pub title: Option<String>,
    pub created_by: Subject,
    pub created_at: WallClock,
    pub bound_at: Option<WallClock>,
    pub ended_at: Option<WallClock>,
    pub ended_by: Option<Subject>,
    pub first_seq: JournalSeq,
    pub last_seq: JournalSeq,
    /// The model's name as the chat header shows it ("Claude Sonnet 5.5"; spec 02 request 8).
    pub model_label: String,
}

/// One row of `GET /api/conversations?site=`: the conversation, what its open turn waits on, and the wall-clock
/// times of its first and last entries, null while it holds none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "conversation.ts")]
pub struct ConversationSummary {
    #[serde(flatten)]
    pub conversation: Conversation,
    pub waiting: Option<WaitingOn>,
    pub first_at: Option<WallClock>,
    pub last_at: Option<WallClock>,
}

/// `GET /api/conversations?site=&before=&limit=`'s answer: the site's live conversation, then its archived ones,
/// newest first; `next`, when more remain, is the `before` that reads them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "conversation.ts")]
pub struct ConversationList {
    pub items: Vec<ConversationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next: Option<JournalSeq>,
}

/// `POST /api/conversations`'s body: a site's root scope for its live conversation, or nothing (`{}`) for the
/// person's new-site thread.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "conversation.ts")]
pub struct OpenConversation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub site: Option<SitePath>,
}

/// `POST /api/conversations`'s answer: `200` for the conversation that was live, `201` for one just made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "conversation.ts")]
pub struct OpenedConversation {
    pub conversation: Conversation,
}

/// `POST /api/conversations/{c}/dismiss`'s answer, the same for a repeat: a site's conversation archived and the
/// site's next live one, or a new-site thread discarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export_to = "conversation.ts")]
pub enum Dismissed {
    Archived { archived: Box<Conversation>, live: Box<Conversation> },
    Discarded { discarded: Box<Conversation> },
}

/// `conversation.changed`'s body (spec 01 section 3.5): a new-site thread bound to its site (`live`, with the site
/// and its title), or the conversation dismissed, `next` naming the site's new live conversation; journaled as the
/// conversation's last entry when it is archived or discarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "conversation.ts")]
pub struct ConversationChanged {
    pub state: ConversationState,
    pub site: Option<SitePath>,
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next: Option<Ulid>,
    pub by: Person,
}
