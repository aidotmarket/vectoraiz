//! The only provider transport. Never log credentials, bodies, or raw errors.
use crate::{
    credentials::Secret,
    index::{Answer, Citation},
};
use reqwest::{redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{sync::OnceLock, time::Duration};
use tokio::sync::watch;
mod mcp;
pub use mcp::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
}
impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::OpenAi => "OpenAI",
        }
    }
    fn host(self) -> &'static str {
        match self {
            Self::Anthropic => "api.anthropic.com",
            Self::OpenAi => "api.openai.com",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    KeyCheck,
    Ask,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Consent {
    pub provider: Provider,
    pub disclosure_version: String,
    pub confirmed_at: String,
}
#[derive(Clone, Default)]
pub struct ConsentState {
    pub enabled: bool,
    pub stopped: bool,
    pub provider: Option<Provider>,
    pub consent: Option<Consent>,
    pub key_present: bool,
}
pub type Epoch = u64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateError {
    ConsentRequired,
    PathOff,
    NoKey,
    Revoked,
}
pub const DISCLOSURE:[&str;6]=[
 "Use your {Provider} API key",
 "When you ask a question, vectorAIz sends your question and the matching passages from your documents to {Provider}. {Provider} writes the answer.",
 "Usage is billed to your own {Provider} account.",
 "You can stop this at any time in Settings. What was already sent can't be taken back.",
 "Use {Provider}","Cancel"];
// FNV-1a over the exact templates, including delimiters, at compile time.
const fn disclosure_hash() -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut i = 0;
    while i < DISCLOSURE.len() {
        let b = DISCLOSURE[i].as_bytes();
        let mut j = 0;
        while j < b.len() {
            h ^= b[j] as u64;
            h = h.wrapping_mul(0x100000001b3);
            j += 1;
        }
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    h
}
pub const DISCLOSURE_VERSION_API_KEY: u64 = disclosure_hash();
pub fn disclosure_version() -> String {
    format!("{DISCLOSURE_VERSION_API_KEY:016x}")
}
pub fn disclosure(p: Provider) -> Value {
    json!({"version":disclosure_version(),"strings":DISCLOSURE.map(|s|s.replace("{Provider}",p.name()))})
}
pub fn stopped_message(p: Provider) -> String {
    format!(
        "vectorAIz has stopped sending to {}. What was already sent can't be taken back.",
        p.name()
    )
}
pub fn check(s: &ConsentState, p: Provider, op: Op) -> Result<(), GateError> {
    if s.stopped {
        return Err(GateError::Revoked);
    }
    if op == Op::Ask && !s.enabled {
        return Err(GateError::PathOff);
    }
    if !s
        .consent
        .as_ref()
        .is_some_and(|c| c.provider == p && c.disclosure_version == disclosure_version())
        || (op == Op::Ask && s.provider != Some(p))
    {
        return Err(GateError::ConsentRequired);
    }
    if op == Op::Ask && !s.key_present {
        return Err(GateError::NoKey);
    }
    Ok(())
}
pub const ALLOWED: [&str; 2] = ["api.anthropic.com", "api.openai.com"];
pub fn allowed(p: Provider, u: &Url) -> bool {
    u.scheme() == "https"
        && u.host_str()
            .is_some_and(|h| ALLOWED.contains(&h) && h == p.host())
        && u.username().is_empty()
        && u.password().is_none()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    ConsentRequired,
    PathOff,
    NoKey,
    Revoked,
    Cancelled,
    NotAllowed,
    KeyRejected,
    Quota,
    RateLimited,
    ModelUnavailable,
    RequestRejected,
    Unreachable,
    ProviderError,
    Incomplete,
    Declined,
    BadCitation,
    QuestionTooLong,
}
impl From<GateError> for SendError {
    fn from(e: GateError) -> Self {
        match e {
            GateError::ConsentRequired => Self::ConsentRequired,
            GateError::PathOff => Self::PathOff,
            GateError::NoKey => Self::NoKey,
            GateError::Revoked => Self::Revoked,
        }
    }
}
impl SendError {
    pub fn reason(self) -> &'static str {
        match self {
            Self::ConsentRequired => "consent_required",
            Self::PathOff => "path_off",
            Self::NoKey => "no_key",
            Self::Revoked => "revoked",
            Self::Cancelled => "cancelled",
            Self::NotAllowed => "request_rejected",
            Self::KeyRejected => "key_rejected",
            Self::Quota => "quota",
            Self::RateLimited => "rate_limited",
            Self::ModelUnavailable => "model_unavailable",
            Self::RequestRejected => "request_rejected",
            Self::Unreachable => "unreachable",
            Self::ProviderError => "provider_error",
            Self::Incomplete => "incomplete",
            Self::Declined => "declined",
            Self::BadCitation => "bad_citation",
            Self::QuestionTooLong => "question_too_long",
        }
    }
    pub fn message(self, p: Provider) -> String {
        let p = p.name();
        match self {
 Self::KeyRejected=>format!("{p} did not accept your API key. Check it in Settings."),
 Self::Quota=>format!("Your {p} account is out of credit or has reached its spending limit. Check billing with {p}."),
 Self::RateLimited=>format!("{p} is limiting requests on your account. Try again in a minute."),
 Self::ModelUnavailable=>format!("vectorAIz needs an update to keep using {p}."),
 Self::RequestRejected|Self::NotAllowed=>format!("{p} couldn't handle this request. Try a shorter question."),
 Self::Unreachable=>format!("Can't reach {p}. Check your connection. Search still works."),
 Self::Incomplete=>format!("{p} stopped before finishing the answer. Try a simpler question."),
 Self::Declined=>format!("{p} declined to answer this question."),
 Self::BadCitation=>format!("{p}'s answer cited a passage that wasn't sent. Try again."),
 Self::PathOff=>"Answers with your API key are off. Turn them on in Settings.".into(),
 Self::NoKey=>format!("Add your {p} API key in Settings."),Self::QuestionTooLong=>"Questions can be up to 2,000 characters.".into(),
 Self::ConsentRequired=>"Review and confirm".into(),Self::Revoked|Self::Cancelled=>"Cancelled".into(),Self::ProviderError=>format!("{p} didn't answer. Try again.")}
    }
}
struct ProviderClient(reqwest::Client);
impl ProviderClient {
    fn builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(120))
    }
    fn new() -> Result<Self, SendError> {
        Self::builder()
            .build()
            .map(Self)
            .map_err(|_| SendError::Unreachable)
    }
    #[cfg(any(test, feature = "test-support"))]
    fn for_test(resolve: std::net::SocketAddr, cert: reqwest::Certificate) -> Self {
        Self(
            Self::builder()
                .no_proxy()
                .resolve(ALLOWED[0], resolve)
                .resolve(ALLOWED[1], resolve)
                .add_root_certificate(cert)
                .build()
                .unwrap(),
        )
    }
    #[cfg(test)]
    fn timed_for_test(resolve: std::net::SocketAddr, cert: reqwest::Certificate) -> Self {
        Self(
            Self::builder()
                .no_proxy()
                .resolve(ALLOWED[0], resolve)
                .resolve(ALLOWED[1], resolve)
                .add_root_certificate(cert)
                .read_timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
        )
    }
    #[cfg(test)]
    fn untrusted_for_test(resolve: std::net::SocketAddr) -> Self {
        Self(
            Self::builder()
                .no_proxy()
                .resolve(ALLOWED[0], resolve)
                .resolve(ALLOWED[1], resolve)
                .build()
                .unwrap(),
        )
    }
}
fn client() -> Result<&'static ProviderClient, SendError> {
    static CLIENT: OnceLock<Result<ProviderClient, SendError>> = OnceLock::new();
    CLIENT
        .get_or_init(ProviderClient::new)
        .as_ref()
        .map_err(|e| *e)
}
pub struct Stream {
    response: reqwest::Response,
    provider: Provider,
}
pub async fn send(
    state_fn: impl Fn() -> (ConsentState, Epoch),
    provider: Provider,
    op: Op,
    key: &Secret,
    body: Option<Value>,
    epoch: &Epoch,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Stream, SendError> {
    dispatch(client()?, state_fn, provider, op, key, body, epoch, cancel).await
}
#[allow(clippy::too_many_arguments)]
async fn dispatch(
    client: &ProviderClient,
    state_fn: impl Fn() -> (ConsentState, Epoch),
    p: Provider,
    op: Op,
    key: &Secret,
    body: Option<Value>,
    epoch: &Epoch,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Stream, SendError> {
    let path = match (p, op) {
        (_, Op::KeyCheck) => "models",
        (Provider::Anthropic, _) => "messages",
        (Provider::OpenAi, _) => "responses",
    };
    let url = Url::parse(&format!("https://{}/v1/{path}", p.host()))
        .map_err(|_| SendError::NotAllowed)?;
    if !allowed(p, &url) {
        return Err(SendError::NotAllowed);
    }
    for attempt in 0..2 {
        let (s, now) = state_fn();
        if *epoch != now {
            return Err(SendError::Revoked);
        }
        // Ask retries retain their original key after a failed replacement;
        // every runtime revocation bumps the epoch (section 2.5).
        if attempt == 0 || op == Op::KeyCheck {
            check(&s, p, op)?;
        }
        if *cancel.borrow() {
            return Err(SendError::Cancelled);
        }
        let r = match op {
            Op::KeyCheck => client.0.get(url.clone()),
            Op::Ask => client.0.post(url.clone()).json(&body),
        };
        let r = match p {
            Provider::Anthropic => r
                .header("x-api-key", key.expose())
                .header("anthropic-version", "2023-06-01"),
            Provider::OpenAi => r.bearer_auth(key.expose()),
        };
        let result = tokio::select! {biased;_=cancel.changed()=>return Err(SendError::Cancelled),r=r.send()=>r};
        let response = match result {
            Ok(r) => r,
            Err(e) => {
                // A connect-phase reset proves that no HTTP response byte was
                // received. Partial-header resets cannot be safely replayed.
                if attempt == 0 && e.is_connect() && connection_reset(&e) {
                    continue;
                }
                return Err(if e.is_connect() {
                    SendError::Unreachable
                } else {
                    SendError::ProviderError
                });
            }
        };
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let retry = response.headers().contains_key("retry-after");
            let b = tokio::select! {biased;_=cancel.changed()=>return Err(SendError::Cancelled),b=response.bytes()=>b.map_err(|_|SendError::ProviderError)?};
            let v = serde_json::from_slice(&b).unwrap_or(Value::Null);
            return Err(map_failure(p, status, retry, &v));
        }
        return Ok(Stream {
            response,
            provider: p,
        });
    }
    Err(SendError::ProviderError)
}
fn connection_reset(e: &reqwest::Error) -> bool {
    use std::error::Error;
    let mut s = e.source();
    while let Some(e) = s {
        if e.downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionReset)
        {
            return true;
        }
        s = e.source();
    }
    false
}
fn map_failure(p: Provider, status: u16, retry: bool, v: &Value) -> SendError {
    let e = &v["error"];
    let kind = e["type"].as_str().unwrap_or("");
    let code = e["code"].as_str().unwrap_or("");
    let msg = e["message"].as_str().unwrap_or("").to_lowercase();
    let quota = [
        "insufficient_quota",
        "credit_balance_exhausted",
        "organization_spend_limit_exceeded",
        "project_spend_limit_exceeded",
        "organization_usage_limit_exceeded",
    ];
    if status == 401 || status == 403 {
        return SendError::KeyRejected;
    }
    if (p == Provider::Anthropic
        && (status == 402
            || (status == 400
                && kind == "invalid_request_error"
                && ["reached your specified", "usage limits", "credit"]
                    .iter()
                    .any(|s| msg.contains(s)))
            || (status == 429
                && (e["details"]["error_code"] == "enforced_spend_limit_reached" || !retry))))
        || (p == Provider::OpenAi
            && status == 429
            && (quota.contains(&kind) || quota.contains(&code)))
    {
        return SendError::Quota;
    }
    if status == 429 || kind == "rate_limit_error" || code == "rate_limit_exceeded" {
        return SendError::RateLimited;
    }
    if status == 404 {
        return SendError::ModelUnavailable;
    }
    if status == 400
        || status == 413
        || (300..400).contains(&status)
        || kind == "model_context_window_exceeded"
        || code == "model_context_window_exceeded"
    {
        return SendError::RequestRejected;
    }
    SendError::ProviderError
}
fn stream_error(p: Provider, value: &Value) -> SendError {
    let error = &value["error"];
    let kind = error["type"].as_str().unwrap_or("");
    let code = error["code"].as_str().unwrap_or("");
    if p == Provider::Anthropic
        && (kind == "billing_error"
            || error["details"]["error_code"] == "enforced_spend_limit_reached")
    {
        return SendError::Quota;
    }
    if p == Provider::OpenAi
        && [
            "insufficient_quota",
            "credit_balance_exhausted",
            "organization_spend_limit_exceeded",
            "project_spend_limit_exceeded",
            "organization_usage_limit_exceeded",
        ]
        .iter()
        .any(|q| *q == kind || *q == code)
    {
        return SendError::Quota;
    }
    map_failure(p, 500, true, value)
}

#[derive(Default)]
struct Parser {
    bytes: Vec<u8>,
    data: Vec<String>,
    event: String,
    stop: Option<String>,
    refusal: bool,
    terminal: bool,
    text: String,
}
impl Parser {
    fn push(
        &mut self,
        bytes: &[u8],
        p: Provider,
        emit: &mut impl FnMut(&str),
    ) -> Result<(), SendError> {
        self.bytes.extend_from_slice(bytes);
        while let Some(end) = self.bytes.iter().position(|b| *b == b'\n') {
            let mut line: Vec<_> = self.bytes.drain(..=end).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8(line).map_err(|_| SendError::ProviderError)?;
            if line.is_empty() {
                self.frame(p, emit)?;
            } else if let Some(d) = line.strip_prefix("data:") {
                self.data.push(d.strip_prefix(' ').unwrap_or(d).into());
            } else if let Some(e) = line.strip_prefix("event:") {
                self.event = e.trim().into();
            }
        }
        Ok(())
    }
    fn frame(&mut self, p: Provider, emit: &mut impl FnMut(&str)) -> Result<(), SendError> {
        let event = std::mem::take(&mut self.event);
        if self.data.is_empty() {
            return Ok(());
        }
        let data = std::mem::take(&mut self.data).join("\n");
        if !event.is_empty()
            && ![
                "content_block_delta",
                "message_delta",
                "message_stop",
                "response.output_text.delta",
                "response.refusal.delta",
                "response.refusal.done",
                "response.completed",
                "response.incomplete",
                "response.failed",
                "error",
            ]
            .contains(&event.as_str())
        {
            return Ok(());
        }
        let v: Value = serde_json::from_str(&data).map_err(|_| SendError::ProviderError)?;
        let kind = v["type"].as_str().unwrap_or(&event);
        let token = match (p, kind) {
            (Provider::Anthropic, "content_block_delta") if v["delta"]["type"] == "text_delta" => {
                v["delta"]["text"].as_str()
            }
            (Provider::OpenAi, "response.output_text.delta") => v["delta"].as_str(),
            _ => None,
        };
        if let Some(t) = token {
            self.text.push_str(t);
            emit(t);
        }
        match (p, kind) {
            (Provider::Anthropic, "message_delta") => {
                self.stop = v["delta"]["stop_reason"].as_str().map(str::to_owned)
            }
            (Provider::Anthropic, "message_stop") => match self.stop.as_deref() {
                Some("end_turn" | "stop_sequence") => self.terminal = true,
                Some("max_tokens") => return Err(SendError::Incomplete),
                Some("refusal") => return Err(SendError::Declined),
                Some("model_context_window_exceeded") => return Err(SendError::RequestRejected),
                _ => return Err(SendError::ProviderError),
            },
            (Provider::OpenAi, "response.refusal.delta" | "response.refusal.done") => {
                self.refusal = true
            }
            (Provider::OpenAi, "response.completed") => {
                if self.refusal
                    || v["response"]["output"].as_array().is_some_and(|items| {
                        items.iter().any(|i| {
                            i["content"]
                                .as_array()
                                .is_some_and(|parts| parts.iter().any(|p| p["type"] == "refusal"))
                        })
                    })
                {
                    return Err(SendError::Declined);
                }
                if self.text.is_empty() {
                    return Err(SendError::ProviderError);
                }
                self.terminal = true;
            }
            (Provider::OpenAi, "response.incomplete") => return Err(SendError::Incomplete),
            (_, "error") => return Err(stream_error(p, &v)),
            (Provider::OpenAi, "response.failed") => return Err(stream_error(p, &v["response"])),
            _ => {}
        }
        Ok(())
    }
}
impl Stream {
    pub async fn answer(
        mut self,
        epoch_valid: impl Fn() -> bool,
        cancel: &mut watch::Receiver<bool>,
        mut emit: impl FnMut(&str),
        citations: Vec<Citation>,
    ) -> Result<Answer, SendError> {
        let mut parser = Parser::default();
        loop {
            if *cancel.borrow() || !epoch_valid() {
                return Err(SendError::Cancelled);
            }
            let bytes = tokio::select! {biased;_=cancel.changed()=>return Err(SendError::Cancelled),b=self.response.chunk()=>b.map_err(|_|SendError::ProviderError)?};
            let Some(bytes) = bytes else {
                return Err(SendError::ProviderError);
            };
            if !epoch_valid() || *cancel.borrow() {
                return Err(SendError::Cancelled);
            }
            parser.push(&bytes, self.provider, &mut emit)?;
            if parser.terminal {
                break;
            }
        }
        if !epoch_valid() || *cancel.borrow() {
            return Err(SendError::Cancelled);
        }
        crate::index::finish_answer(parser.text, citations, emit)
            .map_err(|_| SendError::BadCitation)
    }
}
pub fn request(p: Provider, question: &str, passages: &str) -> Result<Value, SendError> {
    if question.chars().count() > 2000 {
        return Err(SendError::QuestionTooLong);
    }
    let manifest: Value =
        serde_json::from_str(include_str!("../../../spike/fixtures/providers.json"))
            .expect("bundled providers");
    let mut body = manifest[p.id()]["parameters"].clone();
    body["model"] = manifest[p.id()]["model"].clone();
    let system = format!(
        "{} Cite each fact with its source number in square brackets, for example [2].",
        crate::engine::ASK_SYSTEM
    );
    let input = format!("{question}\n\n{passages}");
    match p {
        Provider::Anthropic => {
            body["system"] = json!(system);
            body["messages"] = json!([{"role":"user","content":input}]);
        }
        Provider::OpenAi => {
            body["instructions"] = json!(system);
            body["input"] = json!(input);
        }
    }
    Ok(body)
}
#[cfg(test)]
mod tests;

/// Loopback TLS client for lifecycle tests. Production uses the private singleton.
#[cfg(feature = "test-support")]
pub struct TestTransport(ProviderClient);
#[cfg(feature = "test-support")]
impl TestTransport {
    pub fn new(resolve: std::net::SocketAddr, pem: &[u8]) -> Self {
        assert!(resolve.ip().is_loopback());
        Self(ProviderClient::for_test(
            resolve,
            reqwest::Certificate::from_pem(pem).unwrap(),
        ))
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn send(
        &self,
        state_fn: impl Fn() -> (ConsentState, Epoch),
        provider: Provider,
        op: Op,
        key: &Secret,
        body: Option<Value>,
        epoch: &Epoch,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Stream, SendError> {
        dispatch(&self.0, state_fn, provider, op, key, body, epoch, cancel).await
    }
}
