//! Within `tools`, `ask` is `ask_user` (spec 01 section 3.9; SPEC.md section 5.2, Claude's question shape): one to four
//! questions of two to four options each, which pause the turn on a question card until a person answers, or answers
//! in words with a message. The loop asks and resumes (`agent::calls`); the registry holds the tool for its name,
//! description and schema.

use agent_loop::{Class, Tool, ToolFuture};
use serde_json::{Value, json};
use studio_protocol::{Problem, ProblemType};
use wps_client::wps::Wps;

use super::ToolContext;

pub struct AskUser;

impl<S: Wps> Tool<ToolContext<S>> for AskUser {
    fn name(&self) -> &'static str {
        "ask_user"
    }

    fn class(&self) -> Class {
        Class::Page
    }

    fn description(&self) -> &'static str {
        "Ask the person one to four questions, each with two to four options, and wait for the answer. Use it for \
         discrete choices; it pauses your turn until they answer on the card, or in words with a message. Returns \
         { answers: [{ header, selected: [label], other? }] } or { answered_in_words: true }, the words following."
    }

    fn input_schema(&self) -> Value {
        let option = json!({
            "type": "object", "additionalProperties": false, "required": ["label", "description"],
            "properties": {
                "label": { "type": "string", "minLength": 1, "maxLength": 40 },
                "description": { "type": "string", "maxLength": 160 },
            },
        });
        json!({
            "type": "object", "additionalProperties": false, "required": ["items"],
            "properties": { "items": { "type": "array", "minItems": 1, "maxItems": 4, "items": {
                "type": "object", "additionalProperties": false, "required": ["header", "text", "options"],
                "properties": {
                    "header": { "type": "string", "minLength": 1, "maxLength": 12,
                                "description": "a short topic, 12 characters at most" },
                    "text": { "type": "string", "minLength": 1, "description": "the question in full" },
                    "options": { "type": "array", "minItems": 2, "maxItems": 4, "items": option },
                    "multi": { "type": "boolean", "description": "whether several options may be chosen" },
                    "other": { "type": "boolean" },
                },
            } } },
        })
    }

    fn describe(&self, input: &Value) -> String {
        let count = input["items"].as_array().map_or(0, Vec::len);
        match count {
            1 => "Asking the person a question".to_owned(),
            n => format!("Asking the person {n} questions"),
        }
    }

    fn summarize(&self, _input: &Value, _result: &Value) -> String {
        "Asked the person; they answered".to_owned()
    }

    fn execute<'a>(&'a self, _cx: &'a ToolContext<S>, _input: Value) -> ToolFuture<'a, Value> {
        let problem = Problem::new(ProblemType::Internal, "ask_user waits on the person; the loop asks it.");
        Box::pin(async move { Err(problem) })
    }
}
