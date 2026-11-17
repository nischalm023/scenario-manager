//! Within `studio-protocol`, `question` is the agent's question to the person (spec 01 sections 3.5 and 3.6; spec
//! 02 section 3.1, QuestionCard): what it asks, the answers given on the card or in a message's words, and the body
//! a card posts its answers with.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::grammar::Ulid;
use crate::person::Person;
use crate::projection::{CallId, QuestionState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "question.ts")]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// One item of a question: its short header, its text, whether several options may be chosen, and whether the
/// person may write an answer of their own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "question.ts")]
pub struct QuestionItem {
    pub header: String,
    pub text: String,
    pub multi: bool,
    pub other: bool,
    pub options: Vec<QuestionOption>,
}

/// `question.asked`'s body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "question.ts")]
pub struct QuestionAsked {
    pub question: Ulid,
    pub call: Option<CallId>,
    pub items: Vec<QuestionItem>,
}

/// One item's answer: the item's index, the labels chosen, and the person's own words when they wrote some.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "question.ts")]
pub struct ItemAnswer {
    pub item: u32,
    pub selected: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub other: Option<String>,
}

/// `question.answered`'s body; `in_words` when a message answered it instead of the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "question.ts")]
pub struct QuestionAnswered {
    pub question: Ulid,
    pub answers: Vec<ItemAnswer>,
    pub in_words: bool,
    pub author: Person,
}

/// `question.expired`'s body: the turn was stopped, or its wait ran out (the turn's `end_reason` says which).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "question.ts")]
pub struct QuestionExpired {
    pub question: Ulid,
}

/// `POST /api/questions/{q}/answers`'s body: an answer for every item, with labels the item offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "question.ts")]
pub struct PostAnswers {
    pub answers: Vec<ItemAnswer>,
}

/// A question as an answer's `200` names it: its id and its state once the answer was taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "question.ts")]
pub struct QuestionRef {
    pub id: Ulid,
    pub state: QuestionState,
}

/// `POST /api/questions/{q}/answers`'s `200`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "question.ts")]
pub struct AnswersAccepted {
    pub question: QuestionRef,
}
