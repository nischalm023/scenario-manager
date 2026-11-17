//! Within `config`, `model` is the model's settings (spec 01 sections 3.1 and 3.7, rulings D46 and D122; brief I1.03
//! section 2): the provider the agent's requests go to and its credentials, the model's id per provider and its label,
//! and the request profile's effort, thinking display and output ceiling. Locally the provider is Gemini by default,
//! its key read from a file; Anthropic needs `ANTHROPIC_API_KEY`; a cluster refuses both, and its Vertex route
//! answers messages `not-built` until task 52.

use std::path::PathBuf;
use std::str::FromStr;

use super::{
    ANTHROPIC_API_KEY_VAR, ConfigError, Deployment, MODEL_EFFORT_VAR, MODEL_ID_VAR, MODEL_LABEL_VAR,
    MODEL_MAX_TOKENS_VAR, MODEL_THINKING_DISPLAY_VAR, MODEL_TRANSPORT_VAR, Secret, VERTEX_PROJECT_VAR,
    VERTEX_REGION_VAR, Vars,
};

pub const GEMINI_API_KEY_FILE_VAR: &str = "STUDIO_GEMINI_API_KEY_FILE";

const DEFAULT_MAX_TOKENS: u32 = 32_000;
const GEMINI_MODEL: &str = "gemini-2.5-flash";
const CLAUDE_MODEL: &str = "claude-sonnet-5-5";

/// The model's settings: `id` is the model's id as its provider names it, `label` its name as Settings shows it.
#[derive(Debug, Clone)]
pub struct Model {
    pub transport: ModelTransport,
    pub id: String,
    pub label: String,
    pub effort: Effort,
    pub thinking_display: ThinkingDisplay,
    pub max_tokens: OutputTokens,
}

/// Where the agent's requests go (ruling D122): Gemini with the key the file names, locally the default; Anthropic's
/// API with a key, locally only; or Claude on Vertex AI, every deployment's, not built until task 52.
#[derive(Debug, Clone)]
pub enum ModelTransport {
    Gemini { key_file: PathBuf },
    Anthropic { api_key: Secret },
    Vertex { project: String, region: String },
}

/// The request's `output_config.effort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

/// Whether a request asks for the model's thought summaries (Gemini's `includeThoughts`): `omitted`, the default, asks
/// for none, since the chat never shows them (D117); `updates` asks, and they reach only the transcript and the call's
/// document. Function calling needs the thought signatures alone, which come either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingDisplay {
    Updates,
    Omitted,
}

/// The ceiling on one model request's output, in tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputTokens(u32);

impl OutputTokens {
    pub fn get(self) -> u32 {
        self.0
    }
}

/// Read the model's variables: Gemini by default locally and Vertex in a cluster, which refuses the two key-based
/// providers.
pub(super) fn read(vars: &Vars<'_>, deployment: Deployment) -> Result<Model, ConfigError> {
    let stated = vars.stated(MODEL_TRANSPORT_VAR)?;
    let named = stated.as_deref().unwrap_or(match deployment {
        Deployment::Local => "gemini",
        Deployment::Cluster => "vertex",
    });
    if deployment == Deployment::Cluster && matches!(named, "gemini" | "anthropic") {
        let why = "every deployment reaches the model through Vertex AI";
        return Err(ConfigError::RefusedInCluster { var: MODEL_TRANSPORT_VAR, why });
    }
    let transport = match named {
        "gemini" => ModelTransport::Gemini {
            key_file: PathBuf::from(vars.required_when(GEMINI_API_KEY_FILE_VAR, "STUDIO_MODEL_TRANSPORT is gemini")?),
        },
        "anthropic" => ModelTransport::Anthropic {
            api_key: Secret(vars.required_when(ANTHROPIC_API_KEY_VAR, "STUDIO_MODEL_TRANSPORT is anthropic")?),
        },
        "vertex" => ModelTransport::Vertex {
            project: vars.required_when(VERTEX_PROJECT_VAR, "STUDIO_MODEL_TRANSPORT is vertex")?,
            region: vars.required_when(VERTEX_REGION_VAR, "STUDIO_MODEL_TRANSPORT is vertex")?,
        },
        other => return Err(vars.unreadable(MODEL_TRANSPORT_VAR, other, "gemini, anthropic or vertex")),
    };
    let id = match vars.stated(MODEL_ID_VAR)? {
        Some(id) => id,
        None if matches!(transport, ModelTransport::Gemini { .. }) => GEMINI_MODEL.to_owned(),
        None => CLAUDE_MODEL.to_owned(),
    };
    let label = vars.stated(MODEL_LABEL_VAR)?.unwrap_or_else(|| label_of(&id));
    Ok(Model {
        transport,
        id,
        label,
        effort: vars.read_or(MODEL_EFFORT_VAR, Effort::Medium, |text| text.parse())?,
        thinking_display: vars.read_or(MODEL_THINKING_DISPLAY_VAR, ThinkingDisplay::Omitted, |text| text.parse())?,
        max_tokens: vars.read_or(MODEL_MAX_TOKENS_VAR, OutputTokens(DEFAULT_MAX_TOKENS), |text| text.parse())?,
    })
}

impl FromStr for Effort {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" => Ok(Self::XHigh),
            "max" => Ok(Self::Max),
            _ => Err("low, medium, high, xhigh or max".to_owned()),
        }
    }
}

impl FromStr for ThinkingDisplay {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "updates" => Ok(Self::Updates),
            "omitted" => Ok(Self::Omitted),
            _ => Err("updates or omitted".to_owned()),
        }
    }
}

impl FromStr for OutputTokens {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.parse() {
            Ok(0) | Err(_) => Err("a whole number of tokens above 0".to_owned()),
            Ok(tokens) => Ok(Self(tokens)),
        }
    }
}

/// A model's name as people read it, for the models this build names; another model goes by its id.
fn label_of(id: &str) -> String {
    match id {
        GEMINI_MODEL => "Gemini 2.5 Flash".to_owned(),
        CLAUDE_MODEL => "Claude Sonnet 5.5".to_owned(),
        other => other.to_owned(),
    }
}
