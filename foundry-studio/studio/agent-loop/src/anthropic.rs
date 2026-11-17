//! Within the agent loop, `anthropic` is the Anthropic provider (ruling D122: Anthropic once a key exists): Claude
//! through Anthropic's Messages API with the developer's `ANTHROPIC_API_KEY`, which rig sends in its `x-api-key`
//! header. Claude on Vertex is task 52's, through rig-vertexai, and is not built here.

use rig_core::http_client::DynHttpClient;
use rig_core::providers::anthropic::AnthropicConfig;

use crate::backend::Provider;
use crate::rig_backend::RigBackend;

pub const API_KEY_VAR: &str = "ANTHROPIC_API_KEY";
/// Spec 01 section 3.1's model (ruling D114 amending D113), which the Anthropic and Vertex routes share.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";

/// Anthropic's `model` with `key`, through `http`, at `base_url` when a test stands in for Anthropic's.
pub fn backend(key: &str, model: &str, http: DynHttpClient, base_url: Option<&str>) -> RigBackend {
    let mut config = AnthropicConfig::new(key.to_owned());
    if let Some(base_url) = base_url {
        config = config.with_base_url(base_url);
    }
    let client = config.connect(http);
    RigBackend::new(Provider::Anthropic, model, client.completion(model).erase(), None, vec![key.to_owned()])
}
