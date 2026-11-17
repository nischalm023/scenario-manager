//! Within the agent loop, `gemini` is the Gemini provider (ruling D122, Gemini first; LEAD-DECISIONS entry 50): the
//! key read from the file `STUDIO_GEMINI_API_KEY_FILE` names, its first line, from a file nobody but its owner can
//! read; and the transport that carries it. rig's Gemini wire puts the key in the request's query, where it would
//! reach every log a URL reaches, so the transport takes it out of the query and sends it in the `x-goog-api-key`
//! header, which Gemini reads as well; the key itself never reaches a log, an error or a `Debug` line.

use std::fmt::{self, Debug, Formatter};
use std::fs;
use std::future::Future;
use std::io::Error as IoError;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use http::{HeaderValue, Request, Uri};
use rig_core::http_client::{
    DynHttpClient, HttpClientExt, LazyBody, MultipartForm, Response, Result as HttpResult, StreamingResponse,
};
use rig_core::providers::gemini::GeminiConfig;
use rig_core::wasm_compat::WasmCompatSend;
use serde_json::json;
use thiserror::Error;

use crate::backend::Provider;
use crate::rig_backend::RigBackend;

pub const API_KEY_FILE_VAR: &str = "STUDIO_GEMINI_API_KEY_FILE";
/// Where Gemini's GenerateContent API answers.
pub use rig_core::providers::gemini::BASE_URL;
/// The model probed when D122 was ruled (brief I1.03 section 1).
pub const DEFAULT_MODEL: &str = "gemini-2.5-flash";
const KEY_HEADER: &str = "x-goog-api-key";

/// A Gemini API key, which no `Debug` line shows.
#[derive(Clone)]
pub struct GeminiKey(String);

impl Debug for GeminiKey {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str("GeminiKey(..)")
    }
}

#[derive(Debug, Error)]
pub enum KeyError {
    #[error("{path} cannot be read: {why}")]
    Unreadable { path: PathBuf, why: String },
    #[error("{path} can be read by others (mode {mode:o}); a key file is its owner's alone (chmod 600)")]
    Open { path: PathBuf, mode: u32 },
    #[error("{0} holds no key on its first line")]
    Empty(PathBuf),
}

/// The key on the first line of `path`, refused when the file's group or anyone else may read it.
pub fn read_key(path: &Path) -> Result<GeminiKey, KeyError> {
    let unreadable = |why: IoError| KeyError::Unreadable { path: path.to_owned(), why: why.kind().to_string() };
    let mode = fs::metadata(path).map_err(unreadable)?.permissions().mode() & 0o777;
    if mode & 0o044 != 0 {
        return Err(KeyError::Open { path: path.to_owned(), mode });
    }
    let text = fs::read_to_string(path).map_err(unreadable)?;
    match text.lines().next().map(str::trim) {
        Some(key) if !key.is_empty() => Ok(GeminiKey(key.to_owned())),
        _ => Err(KeyError::Empty(path.to_owned())),
    }
}

/// An HTTP client that sends the Gemini key in its header, never in the URL.
#[derive(Clone)]
pub struct KeyInHeader {
    inner: DynHttpClient,
    key: HeaderValue,
}

impl KeyInHeader {
    pub fn new(inner: DynHttpClient, key: &GeminiKey) -> Self {
        let mut key = HeaderValue::from_str(&key.0).unwrap_or_else(|_| HeaderValue::from_static(""));
        key.set_sensitive(true);
        Self { inner, key }
    }

    fn moved<T>(&self, mut request: Request<T>) -> Request<T> {
        let uri = request.uri();
        if let Some(query) = uri.query() {
            let kept: Vec<&str> = query.split('&').filter(|pair| !pair.starts_with("key=")).collect();
            let path = match kept.is_empty() {
                true => uri.path().to_owned(),
                false => format!("{}?{}", uri.path(), kept.join("&")),
            };
            let mut parts = uri.clone().into_parts();
            parts.path_and_query = path.parse().ok();
            if let Ok(moved) = Uri::from_parts(parts) {
                *request.uri_mut() = moved;
            }
        }
        request.headers_mut().insert(KEY_HEADER, self.key.clone());
        request
    }
}

impl Debug for KeyInHeader {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str("KeyInHeader(..)")
    }
}

impl HttpClientExt for KeyInHeader {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = HttpResult<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.inner.send(self.moved(request))
    }

    fn send_multipart<U>(
        &self,
        request: Request<MultipartForm>,
    ) -> impl Future<Output = HttpResult<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.inner.send_multipart(self.moved(request))
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = HttpResult<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        self.inner.send_streaming(self.moved(request))
    }
}

/// Gemini's `model` with `key`, through `http` (the process-wide reqwest client unless a test names another), at
/// `base_url` when a test or a recording stands in for Google's, asking for the model's thought summaries when
/// `thoughts`; they stay in the transcript and the call's document, since the chat never shows them (D117). The thought
/// signatures function calling needs come on the reply's other parts whether or not it asks.
pub fn backend(
    key: &GeminiKey,
    model: &str,
    http: DynHttpClient,
    base_url: Option<&str>,
    thoughts: bool,
) -> RigBackend {
    let mut config = GeminiConfig::new(key.0.clone());
    if let Some(base_url) = base_url {
        config = config.with_base_url(base_url);
    }
    let client = config.connect(KeyInHeader::new(http, key));
    let params = thoughts.then(|| json!({ "generationConfig": { "thinkingConfig": { "includeThoughts": true } } }));
    RigBackend::new(Provider::Gemini, model, client.completion(model).erase(), params, vec![key.0.clone()])
}
