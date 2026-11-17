//! Brief I1.03's "Done when", the backends' half: each provider's recorded replies replay offline into the events the
//! loop reads (a text block's fragments and its end, the model's reasoning, parallel calls, a length stop, a 429
//! asked again after the wait the schedule gives, then success), and no key reaches a failure's words, a request's URL
//! or a fixture. Gemini's hand-written streams and Anthropic's run through a canned transport; one live Gemini turn,
//! recorded once through rig-cassette's proxy with the key the file names (`RIG_PROVIDER_TEST_MODE=record`), replays
//! from `fixtures/cassettes/gemini/agent-turn.yaml`.

use std::collections::VecDeque;
use std::fs::{self, Permissions};
use std::future::{self, Future};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::{env, process};

use agent_loop::gemini::{self, GeminiKey, KeyError, read_key};
use agent_loop::retry::RetrySchedule;
use agent_loop::rig::{Message, ToolDefinition, ToolName};
use agent_loop::{
    Block, ErrorClass, ModelBackend, ModelError, ModelRequest, StepEnd, StepEvent, Stop, Tokens, anthropic, shared_http,
};
use bytes::Bytes;
use futures::StreamExt;
use futures::stream;
use http::{HeaderMap, HeaderValue, Request, Response, StatusCode};
use rig_cassette::http::{CassetteMode, CassetteSpec, ProviderCassette, RecordVia};
use rig_core::http_client::{
    DynHttpClient, Error as HttpError, HttpClientExt, LazyBody, MultipartForm, Result as HttpResult, StreamingResponse,
};
use rig_core::wasm_compat::WasmCompatSend;
use serde_json::json;

const KEY: &str = "AIzaSyTESTONLY-not-a-real-key-0123456789";
const KEY_FILE: &str = "agent-studio/secrets/google-api-keys.txt";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// One reply a canned transport gives: its status, its headers and its body.
struct Canned {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: String,
}

/// A transport that answers each request with the next canned reply and keeps what each request carried.
#[derive(Clone, Default)]
struct CannedHttp {
    replies: Arc<Mutex<VecDeque<Canned>>>,
    seen: Arc<Mutex<Vec<(String, HeaderMap)>>>,
}

impl CannedHttp {
    fn new(replies: Vec<Canned>) -> Self {
        Self { replies: Arc::new(Mutex::new(replies.into())), seen: Arc::default() }
    }
}

impl HttpClientExt for CannedHttp {
    fn send<T, U>(
        &self,
        _request: Request<T>,
    ) -> impl Future<Output = HttpResult<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        future::ready(Err(HttpError::non_success_with_details(
            StatusCode::NOT_IMPLEMENTED,
            HeaderMap::new(),
            String::new(),
        )))
    }

    fn send_multipart<U>(
        &self,
        _request: Request<MultipartForm>,
    ) -> impl Future<Output = HttpResult<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        future::ready(Err(HttpError::non_success_with_details(
            StatusCode::NOT_IMPLEMENTED,
            HeaderMap::new(),
            String::new(),
        )))
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = HttpResult<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        self.seen.lock().expect("the log").push((request.uri().to_string(), request.headers().clone()));
        let reply = self.replies.lock().expect("the replies").pop_front().expect("a canned reply for every request");
        let mut headers = HeaderMap::new();
        for (name, value) in reply.headers {
            headers.insert(name, HeaderValue::from_static(value));
        }
        let status = StatusCode::from_u16(reply.status).expect("a status");
        let answer = if status.is_success() {
            let body: StreamingResponse = Response::builder()
                .status(status)
                .header("content-type", "text/event-stream")
                .body(Box::pin(stream::iter([Ok(Bytes::from(reply.body))])) as _)
                .expect("a response");
            Ok(body)
        } else {
            Err(HttpError::non_success_with_details(status, headers, reply.body))
        };
        future::ready(answer)
    }
}

fn sse(provider: &str, name: &str) -> Canned {
    Canned {
        status: 200,
        headers: Vec::new(),
        body: fs::read_to_string(fixtures().join(provider).join(name)).expect("a fixture"),
    }
}

fn rate_limited(provider: &str) -> Canned {
    let body = fs::read_to_string(fixtures().join(provider).join("rate-limited.json")).expect("a fixture");
    Canned { status: 429, headers: vec![("retry-after", "1")], body }
}

/// A key file of its own holding `KEY`, with `mode`.
fn key_file(mode: u32) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = env::temp_dir().join(format!("agent-loop-key-{}-{n}-{mode:o}", process::id()));
    fs::write(&path, format!("{KEY}\nsecond-key-unused\n")).expect("the key file");
    fs::set_permissions(&path, Permissions::from_mode(mode)).expect("its mode");
    path
}

fn test_key() -> GeminiKey {
    read_key(&key_file(0o600)).expect("a key its owner alone reads")
}

fn tools() -> Vec<ToolDefinition> {
    let tool = |name: &str, description: &str, schema| {
        ToolDefinition::new(ToolName::new(name).expect("a name"), description, schema)
    };
    vec![
        tool(
            "site_list",
            "List the sites.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "template_list",
            "List the site templates, those whose title holds `filter`.",
            json!({
            "type": "object", "properties": { "filter": { "type": "string" } }, "additionalProperties": false }),
        ),
    ]
}

fn request(words: &str) -> ModelRequest {
    ModelRequest {
        system: "You are a test of Agent Studio's agent loop. Follow the user's instructions exactly.".into(),
        history: vec![Message::user(words)],
        tools: tools(),
        tools_off: false,
        max_tokens: Tokens(2_048),
    }
}

/// What one request's events say, in order: each finished block, the text the fragments added up to per block,
/// and the end, or the failure.
#[derive(Debug, Default)]
struct Heard {
    blocks: Vec<Block>,
    fragments: Vec<(u32, String)>,
    end: Option<StepEnd>,
    failed: Option<ModelError>,
}

async fn heard(backend: &dyn ModelBackend, request: ModelRequest) -> Heard {
    let mut events = backend.step(request);
    let mut heard = Heard::default();
    while let Some(event) = events.next().await {
        match event {
            Ok(StepEvent::Delta { part, text }) => match heard.fragments.iter_mut().find(|(at, _)| *at == part) {
                Some((_, held)) => held.push_str(&text),
                None => heard.fragments.push((part, text)),
            },
            Ok(StepEvent::Done { block, .. }) => heard.blocks.push(block),
            Ok(StepEvent::Ended(end)) => heard.end = Some(*end),
            Err(error) => heard.failed = Some(error),
        }
    }
    heard
}

/// A reply's blocks in order as the loop reads them: its reasoning and texts, then each call's tool and arguments.
fn blocks(heard: &Heard) -> Vec<String> {
    (heard.blocks.iter())
        .map(|block| match block {
            Block::Text(text) => format!("text: {text}"),
            Block::Reasoning(thought) => format!("reasoning: {thought}"),
            Block::Call(call) => format!("call: {} {}", call.function.name, json!(call.function.arguments)),
            Block::Other => "other".to_owned(),
        })
        .collect()
}

/// The reasoning, the text in two fragments and the two parallel calls every provider's first fixture holds.
fn the_reply() -> Vec<String> {
    vec![
        "reasoning: The person wants both lists, so I will call both tools together.".into(),
        "text: Looking at the sites and the templates now.".into(),
        "call: site_list {}".into(),
        "call: template_list {\"filter\":\"L\"}".into(),
    ]
}

/// Each provider's fixtures replayed into the journal's events, with `key` in no error text and no URL.
async fn replays_like_the_journal(provider: &str, key: &str, backend: impl Fn(DynHttpClient) -> Box<dyn ModelBackend>) {
    let canned = CannedHttp::new(vec![
        sse(provider, "note-text-calls.sse"),
        sse(provider, "length.sse"),
        rate_limited(provider),
        sse(provider, "text.sse"),
    ]);
    let backend = backend(DynHttpClient::new(canned.clone()));
    let first = heard(backend.as_ref(), request("List both")).await;
    assert_eq!(blocks(&first), the_reply(), "{provider}");
    assert_eq!(first.fragments, [(1, "Looking at the sites and the templates now.".to_owned())], "{provider}");
    let end = first.end.expect("the reply ended");
    assert_eq!((end.stop, end.calls.len(), end.usage.input_tokens), (Stop::ToolUse, 2, Tokens(812)), "{provider}");
    assert!(end.message.is_some() && !end.raw.is_null(), "{provider}: the message to append and the document");

    let cut = heard(backend.as_ref(), request("Describe it")).await;
    assert_eq!(blocks(&cut), ["text: The L-Shape holds six aisles of racks, and each aisle"], "{provider}");
    assert_eq!(cut.end.expect("ended").stop, Stop::MaxTokens, "{provider}");

    let refused = heard(backend.as_ref(), request("Again")).await;
    let error = refused.failed.expect("a 429");
    assert_eq!((error.class, error.status), (ErrorClass::Unavailable, Some(429)), "{provider}: {error}");
    assert!(!error.words.contains(key) && !error.words.contains("http"), "{provider}: {}", error.words);
    let wait = RetrySchedule::default().wait_after(0, &error).expect("asked again");
    assert!(wait.as_secs() >= 2, "{provider}: {wait:?}");
    let again = heard(backend.as_ref(), request("Again")).await;
    assert_eq!(blocks(&again), ["text: Hello again."], "{provider}");
    assert_eq!(again.end.expect("ended").stop, Stop::EndTurn, "{provider}");

    for (uri, _) in canned.seen.lock().expect("the log").iter() {
        assert!(!uri.contains(key) && !uri.contains("key="), "{provider}: a key in {uri}");
    }
}

/// Gemini's hand-written streams: the key travels in `x-goog-api-key`, never in the URL rig puts it in.
#[tokio::test]
async fn gemini_streams_replay_into_the_journals_events() {
    let key = test_key();
    replays_like_the_journal("gemini", KEY, |http| {
        Box::new(gemini::backend(&key, gemini::DEFAULT_MODEL, http, Some("https://gemini.test"), true))
    })
    .await;
    let canned = CannedHttp::new(vec![sse("gemini", "text.sse")]);
    let backend = gemini::backend(
        &key,
        gemini::DEFAULT_MODEL,
        DynHttpClient::new(canned.clone()),
        Some("https://gemini.test"),
        true,
    );
    heard(&backend, request("Hello")).await;
    let seen = canned.seen.lock().expect("the log");
    let (uri, headers) = &seen[0];
    assert_eq!(uri, "https://gemini.test/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse");
    assert_eq!(headers.get("x-goog-api-key").map(HeaderValue::as_bytes), Some(KEY.as_bytes()));
    assert!(headers.get("x-goog-api-key").is_some_and(HeaderValue::is_sensitive));
}

/// Anthropic's hand-written stream, until a key exists to record one.
#[tokio::test]
async fn anthropic_streams_replay_into_the_journals_events() {
    let key = "sk-ant-test-only";
    replays_like_the_journal("anthropic", key, |http| {
        Box::new(anthropic::backend(key, anthropic::DEFAULT_MODEL, http, Some("https://anthropic.test")))
    })
    .await;
}

/// Astra's agent-loop review, the key's finding: a key the provider echoes reaches no failure's words, no stop's reason
/// and no string of the reply's document (`model_calls.raw`), each replaced before the loop reads it.
#[tokio::test]
async fn a_key_the_provider_echoes_reaches_no_error_stop_or_document() {
    let key = test_key();
    let echoed = |status: u16, body: String| Canned { status, headers: Vec::new(), body };
    let rejected = json!({ "error": { "code": 400, "message": format!("API key not valid: {KEY}"), "status": "INVALID_ARGUMENT" } });
    let chunk = |reason: &str, words: &str| {
        let candidate = json!({ "content": { "parts": [{ "text": words }], "role": "model" }, "finishReason": reason,
                                "finishMessage": format!("served for {KEY}"), "index": 0 });
        let usage = json!({ "promptTokenCount": 40, "candidatesTokenCount": 3, "totalTokenCount": 43 });
        let body = json!({ "candidates": [candidate], "usageMetadata": usage, "modelVersion": format!("gemini-2.5-flash-{KEY}"),
                           "responseId": format!("echo-{KEY}") });
        format!("data: {body}\n\n")
    };
    let canned = CannedHttp::new(vec![
        echoed(400, rejected.to_string()),
        echoed(200, chunk("STOP", "Hello.")),
        echoed(200, chunk(KEY, "Hm.")),
    ]);
    let http = DynHttpClient::new(canned);
    let backend = gemini::backend(&key, gemini::DEFAULT_MODEL, http, Some("https://gemini.test"), true);
    let failed = heard(&backend, request("Hello")).await.failed.expect("a 400");
    assert_eq!((failed.class, failed.status), (ErrorClass::Rejected, Some(400)));
    assert!(failed.words.contains("API key not valid") && !failed.words.contains(KEY), "{}", failed.words);
    let end = heard(&backend, request("Hello")).await.end.expect("a reply");
    assert!(!end.raw.is_null() && !end.raw.to_string().contains(KEY), "{}", end.raw);
    let odd = heard(&backend, request("Hello")).await.end.expect("a reply");
    assert!(matches!(&odd.stop, Stop::Other(reason) if !reason.contains(KEY)), "{:?}", odd.stop);
    assert!(!odd.raw.to_string().contains(KEY), "{}", odd.raw);
}

/// LEAD-DECISIONS entry 50: the key is the first line of a file nobody but its owner can read; one others can read is
/// refused, and neither the key nor the file's contents reach the refusal or a `Debug` line.
#[test]
fn a_key_file_others_can_read_is_refused() {
    let open = key_file(0o644);
    let refused = read_key(&open).expect_err("refused");
    assert!(matches!(refused, KeyError::Open { mode: 0o644, .. }), "{refused}");
    assert!(!refused.to_string().contains(KEY));
    assert_eq!(format!("{:?}", test_key()), "GeminiKey(..)");
    let missing = read_key(Path::new("/nonexistent/keys.txt")).expect_err("no file");
    assert!(matches!(missing, KeyError::Unreadable { .. }), "{missing}");
}

/// Whether this run records the live turn: `RIG_PROVIDER_TEST_MODE=record` and the key file in place.
fn recording() -> Option<PathBuf> {
    let asked = env::var("RIG_PROVIDER_TEST_MODE").is_ok_and(|mode| mode.eq_ignore_ascii_case("record"));
    let file = env::var_os("STUDIO_GEMINI_API_KEY_FILE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| Path::new(&home).join(".config").join(KEY_FILE)))?;
    (asked && file.exists()).then_some(file)
}

/// One live Gemini turn, recorded once through rig-cassette's proxy with the file's key and scrubbed, replays offline
/// into the model's reasoning, the text, and two calls made together; the fixture holds no key.
#[tokio::test]
async fn the_recorded_gemini_turn_replays_offline() {
    let fixture = fixtures().join("cassettes/gemini/agent-turn.yaml");
    let live = recording();
    let (mode, key) = match &live {
        Some(file) => (CassetteMode::Record, read_key(file).expect("the key file")),
        None => (CassetteMode::Replay, test_key()),
    };
    let cassette = ProviderCassette::try_start_at(
        RecordVia::Proxy,
        "gemini",
        CassetteSpec::new("agent-turn"),
        gemini::BASE_URL,
        mode,
        fixture.clone(),
    )
    .await
    .expect("the cassette starts");
    let backend = gemini::backend(&key, gemini::DEFAULT_MODEL, shared_http(), Some(&cassette.base_url()), true);
    let words = "Before any tool, write one short sentence saying what you will do. Then, in the same reply, call \
                 site_list and template_list together, template_list with the filter \"L\".";
    let turn = heard(&backend, request(words)).await;
    cassette.try_finish().await.expect("the cassette finishes");
    let said = blocks(&turn);
    assert!(said.iter().any(|line| line.starts_with("text: ")), "{said:?}");
    assert!(said.iter().any(|line| line.starts_with("reasoning: ")), "{said:?}");
    let calls: Vec<&String> = said.iter().filter(|line| line.starts_with("call: ")).collect();
    assert_eq!(calls, ["call: site_list {}", "call: template_list {\"filter\":\"L\"}"], "{said:?}");
    assert_eq!(turn.end.expect("ended").stop, Stop::ToolUse);
    let written = fs::read_to_string(&fixture).expect("the fixture");
    assert!(!written.contains(KEY), "the test key is in the fixture");
    if let Some(file) = live {
        let key = fs::read_to_string(file).expect("the key file");
        let first = key.lines().next().unwrap_or_default().trim();
        assert!(!first.is_empty() && !written.contains(first), "the fixture holds the live key");
    }
}
