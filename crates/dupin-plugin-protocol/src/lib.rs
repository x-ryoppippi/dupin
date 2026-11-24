//! Experimental runtime-independent wire boundary, not a stable external SDK.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::HashSet, fmt, io};

pub const VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub const MAX_RESULTS: u32 = 500;
pub const MAX_QUERY_BYTES: usize = 4096;
pub const MAX_STATE_BYTES: usize = 8192;
/// Encoded bytes of the context a `push` effect hands to the view it opens.
pub const MAX_CONTEXT_BYTES: usize = 4096;
/// Nested arrays and objects allowed on one path inside a view context.
pub const MAX_CONTEXT_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub request_id: String,
    pub generation: String,
    pub command: String,
    pub query: String,
    pub limit: u32,
    pub state: Option<State>,
    /// The view a `push` effect opened, or none for the command's own results. Absent on the
    /// wire when none, so a request outside a view is exactly a version 1 request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// The context the `push` effect carried, given back unchanged with every query of its view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub request_id: String,
    pub generation: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Results {
        items: Vec<PluginItem>,
        state: Option<State>,
    },
    /// One long text with its own actions, as the whole of a pushed view. Only a request for a
    /// view may be answered this way.
    Detail {
        title: String,
        body: String,
        actions: Vec<PluginAction>,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidQuery,
    UnsupportedCommand,
    ResourceLimit,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginItem {
    pub id: String,
    pub view: View,
    pub actions: Vec<PluginAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum View {
    Text { title: String, subtitle: String },
    Detail { title: String, body: String },
    Pair { source: String, result: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginAction {
    pub id: String,
    pub title: String,
    pub effect: Effect,
}

/// What an action does when chosen. The host carries it out; there is no other operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    Copy {
        text: String,
    },
    OpenUrl {
        url: String,
    },
    /// Open the guest's view `view` in the panel, titled `title`. The host queries the same
    /// guest again with the view and `context`.
    Push {
        view: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<serde_json::Value>,
    },
    /// Run the JavaScript guest's `actions[action]` with `data`, because the user chose it.
    /// Only the extension that offered it runs it.
    Run {
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
}

pub const MAX_URL_BYTES: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    MessageTooLarge,
    InvalidJson,
    UnsupportedVersion,
    InvalidField(&'static str),
    CorrelationMismatch,
    TooManyResults,
    DuplicateId(&'static str),
    EncodingFailed,
}
impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(f, "invalid field: {field}"),
            Self::DuplicateId(field) => write!(f, "duplicate identifier: {field}"),
            Self::MessageTooLarge => f.write_str("message exceeds byte limit"),
            Self::InvalidJson => f.write_str("invalid JSON structure"),
            Self::UnsupportedVersion => f.write_str("unsupported protocol version"),
            Self::CorrelationMismatch => f.write_str("response correlation mismatch"),
            Self::TooManyResults => f.write_str("too many results"),
            Self::EncodingFailed => f.write_str("JSON encoding failed"),
        }
    }
}
impl std::error::Error for ProtocolError {}

impl Request {
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let value: Self = decode(bytes, MAX_REQUEST_BYTES)?;
        value.validate()?;
        Ok(value)
    }
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        self.validate()?;
        encode_json(self, MAX_REQUEST_BYTES)
    }
    fn validate(&self) -> Result<(), ProtocolError> {
        version(self.version)?;
        identifier(&self.request_id, "request_id")?;
        identifier(&self.generation, "generation")?;
        identifier(&self.command, "command")?;
        if self.query.len() > MAX_QUERY_BYTES {
            return Err(ProtocolError::InvalidField("query"));
        }
        if self.limit > MAX_RESULTS {
            return Err(ProtocolError::InvalidField("limit"));
        }
        match &self.view {
            Some(view) => identifier(view, "view")?,
            None if self.context.is_some() => return Err(ProtocolError::InvalidField("context")),
            None => {}
        }
        context(&self.context, "context")?;
        state(&self.state)
    }
}
impl Response {
    pub fn decode(bytes: &[u8], request: &Request) -> Result<Self, ProtocolError> {
        let value: Self = decode(bytes, MAX_RESPONSE_BYTES)?;
        value.validate(request)?;
        Ok(value)
    }
    pub fn encode(&self, request: &Request) -> Result<Vec<u8>, ProtocolError> {
        self.validate(request)?;
        encode_json(self, MAX_RESPONSE_BYTES)
    }
    fn validate(&self, request: &Request) -> Result<(), ProtocolError> {
        request.validate()?;
        version(self.version)?;
        identifier(&self.request_id, "request_id")?;
        identifier(&self.generation, "generation")?;
        if self.request_id != request.request_id || self.generation != request.generation {
            return Err(ProtocolError::CorrelationMismatch);
        }
        match &self.outcome {
            Outcome::Error { message, .. } => bounded(message, 2048, "message"),
            Outcome::Results { items, state: saved } => {
                if items.len() > request.limit as usize {
                    return Err(ProtocolError::TooManyResults);
                }
                let mut ids = HashSet::new();
                for item in items {
                    identifier(&item.id, "item.id")?;
                    if !ids.insert(&item.id) {
                        return Err(ProtocolError::DuplicateId("item.id"));
                    }
                    match &item.view {
                        View::Text { title, subtitle } => {
                            bounded(title, 512, "title")?;
                            if subtitle.len() > 2048 {
                                return Err(ProtocolError::InvalidField("subtitle"));
                            }
                        }
                        View::Detail { title, body } => {
                            bounded(title, 512, "title")?;
                            bounded(body, 64 * 1024, "body")?;
                        }
                        View::Pair { source, result } => {
                            bounded(source, 4096, "source")?;
                            bounded(result, 16 * 1024, "result")?;
                        }
                    }
                    actions(&item.actions)?;
                }
                state(saved)
            }
            Outcome::Detail {
                title,
                body,
                actions: offered,
            } => {
                if request.view.is_none() {
                    return Err(ProtocolError::InvalidField("outcome"));
                }
                bounded(title, 512, "title")?;
                bounded(body, 64 * 1024, "body")?;
                actions(offered)
            }
        }
    }
}
fn actions(actions: &[PluginAction]) -> Result<(), ProtocolError> {
    if actions.len() > 8 {
        return Err(ProtocolError::InvalidField("actions"));
    }
    let mut ids = HashSet::new();
    for action in actions {
        identifier(&action.id, "action.id")?;
        bounded(&action.title, 512, "action.title")?;
        match &action.effect {
            Effect::Copy { text } => bounded(text, 16 * 1024, "effect.text")?,
            Effect::OpenUrl { url } => web_url(url)?,
            Effect::Push {
                view,
                title,
                context: carried,
            } => {
                identifier(view, "effect.view")?;
                printable(title, 512, "effect.title")?;
                context(carried, "effect.context")?;
            }
            Effect::Run { action, data } => {
                printable(action, 64, "effect.action")?;
                context(data, "effect.data")?;
            }
        }
        if !ids.insert(&action.id) {
            return Err(ProtocolError::DuplicateId("action.id"));
        }
    }
    Ok(())
}
fn version(value: u32) -> Result<(), ProtocolError> {
    if value == VERSION {
        Ok(())
    } else {
        Err(ProtocolError::UnsupportedVersion)
    }
}
fn bounded(value: &str, cap: usize, field: &'static str) -> Result<(), ProtocolError> {
    if value.is_empty() || value.len() > cap {
        Err(ProtocolError::InvalidField(field))
    } else {
        Ok(())
    }
}
fn identifier(value: &str, field: &'static str) -> Result<(), ProtocolError> {
    printable(value, 128, field)
}
fn printable(value: &str, cap: usize, field: &'static str) -> Result<(), ProtocolError> {
    bounded(value, cap, field)?;
    if value.chars().any(char::is_control) {
        return Err(ProtocolError::InvalidField(field));
    }
    Ok(())
}
fn web_url(url: &str) -> Result<(), ProtocolError> {
    if is_web_url(url) {
        Ok(())
    } else {
        Err(ProtocolError::InvalidField("effect.url"))
    }
}
/// Whether an `open_url` effect may carry `url`: an `http` or `https` URL with a host, at most
/// [`MAX_URL_BYTES`] long, without whitespace or control characters.
pub fn is_web_url(url: &str) -> bool {
    if url.is_empty() || url.len() > MAX_URL_BYTES {
        return false;
    }
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &url[8..]
    } else if lower.starts_with("http://") {
        &url[7..]
    } else {
        return false;
    };
    !(rest.is_empty()
        || rest.starts_with(['/', '?', '#'])
        || url.chars().any(|ch| ch.is_control() || ch.is_whitespace()))
}
fn state(value: &Option<State>) -> Result<(), ProtocolError> {
    if let Some(value) = value {
        if value.version == 0 {
            return Err(ProtocolError::InvalidField("state.version"));
        }
        json_value(&value.data, 64, MAX_STATE_BYTES, "state.data")?;
    }
    Ok(())
}
fn context(value: &Option<serde_json::Value>, field: &'static str) -> Result<(), ProtocolError> {
    match value {
        Some(value) => json_value(value, MAX_CONTEXT_DEPTH, MAX_CONTEXT_BYTES, field),
        None => Ok(()),
    }
}
fn json_value(
    value: &serde_json::Value,
    max_depth: usize,
    cap: usize,
    field: &'static str,
) -> Result<(), ProtocolError> {
    fn depth(value: &serde_json::Value, remaining: usize) -> bool {
        match value {
            serde_json::Value::Array(values) => {
                remaining > 0 && values.iter().all(|v| depth(v, remaining - 1))
            }
            serde_json::Value::Object(values) => {
                remaining > 0 && values.values().all(|v| depth(v, remaining - 1))
            }
            _ => true,
        }
    }
    if !depth(value, max_depth) {
        return Err(ProtocolError::InvalidField(field));
    }
    encode_json(value, cap).map_err(|_| ProtocolError::InvalidField(field))?;
    Ok(())
}
fn decode<T: DeserializeOwned>(bytes: &[u8], cap: usize) -> Result<T, ProtocolError> {
    if bytes.len() > cap {
        return Err(ProtocolError::MessageTooLarge);
    }
    #[derive(Deserialize)]
    struct Envelope {
        version: u32,
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidJson)?;
    version(envelope.version)?;
    serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidJson)
}
struct CappedWriter {
    bytes: Vec<u8>,
    cap: usize,
    exceeded: bool,
}
impl io::Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.cap - self.bytes.len() {
            self.exceeded = true;
            return Err(io::Error::other("byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode_json(value: &impl Serialize, cap: usize) -> Result<Vec<u8>, ProtocolError> {
    let mut writer = CappedWriter {
        bytes: Vec::new(),
        cap,
        exceeded: false,
    };
    if serde_json::to_writer(&mut writer, value).is_err() {
        return Err(if writer.exceeded {
            ProtocolError::MessageTooLarge
        } else {
            ProtocolError::EncodingFailed
        });
    }
    Ok(writer.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> Request {
        Request {
            version: VERSION,
            request_id: "opaque-9007199254740993".into(),
            generation: "世代".into(),
            command: "calculator".into(),
            query: "1 + 2".into(),
            limit: 3,
            state: Some(State {
                version: 1,
                data: json!({"記憶": [true, 3]}),
            }),
            view: None,
            context: None,
        }
    }
    fn response() -> Response {
        Response {
            version: VERSION,
            request_id: request().request_id,
            generation: request().generation,
            outcome: Outcome::Results {
                items: vec![
                    PluginItem {
                        id: "text".into(),
                        view: View::Text {
                            title: "日本語 🦀".into(),
                            subtitle: "A subtitle".into(),
                        },
                        actions: vec![
                            PluginAction {
                                id: "copy".into(),
                                title: "Copy result".into(),
                                effect: Effect::Copy {
                                    text: "日本語 🦀".into(),
                                },
                            },
                            PluginAction {
                                id: "open".into(),
                                title: "Open".into(),
                                effect: Effect::OpenUrl {
                                    url: "https://example.com/?q=%F0%9F%A6%80".into(),
                                },
                            },
                        ],
                    },
                    PluginItem {
                        id: "detail".into(),
                        view: View::Detail {
                            title: "Details".into(),
                            body: "line one\nline two".into(),
                        },
                        actions: vec![],
                    },
                    PluginItem {
                        id: "pair".into(),
                        view: View::Pair {
                            source: "1 + 2".into(),
                            result: "3".into(),
                        },
                        actions: vec![],
                    },
                ],
                state: request().state,
            },
        }
    }
    fn items(value: &mut Response) -> &mut Vec<PluginItem> {
        let Outcome::Results { items, .. } = &mut value.outcome else {
            panic!()
        };
        items
    }
    #[test]
    fn round_trips_all_views_and_errors() {
        let req = request();
        assert_eq!(Request::decode(&req.encode().unwrap()).unwrap(), req);
        let mut res = response();
        assert_eq!(Response::decode(&res.encode(&req).unwrap(), &req).unwrap(), res);
        for code in [
            ErrorCode::InvalidQuery,
            ErrorCode::UnsupportedCommand,
            ErrorCode::ResourceLimit,
            ErrorCode::Internal,
        ] {
            res.outcome = Outcome::Error {
                code,
                message: "Cannot evaluate expression".into(),
            };
            assert_eq!(Response::decode(&res.encode(&req).unwrap(), &req).unwrap(), res);
        }
    }
    #[test]
    fn text_rows_can_omit_visible_subtitle_content() {
        let req = request();
        let mut res = response();
        let View::Text { subtitle, .. } = &mut items(&mut res)[0].view else {
            panic!()
        };
        subtitle.clear();
        assert_eq!(Response::decode(&res.encode(&req).unwrap(), &req).unwrap(), res);
    }

    #[test]
    fn state_depth_counts_containers_without_rejecting_boundary_leaves() {
        let nested = |count, leaf| (0..count).fold(leaf, |value, _| json!([value]));
        let mut req = request();
        req.state.as_mut().unwrap().data = nested(64, serde_json::Value::Null);
        assert_eq!(Request::decode(&req.encode().unwrap()).unwrap(), req);
        for empty in [json!([]), json!({})] {
            req.state.as_mut().unwrap().data = nested(63, empty.clone());
            assert!(req.encode().is_ok());
            req.state.as_mut().unwrap().data = nested(64, empty);
            assert_eq!(req.encode(), Err(ProtocolError::InvalidField("state.data")));
            let bytes = serde_json::to_vec(&req).unwrap();
            assert_eq!(
                Request::decode(&bytes),
                Err(ProtocolError::InvalidField("state.data"))
            );
        }
    }
    #[test]
    fn rejects_malformed_oversize_and_unknown_versions() {
        for bytes in [b"{".as_slice(), b"\xff", b"null", b"{}"] {
            assert_eq!(Request::decode(bytes), Err(ProtocolError::InvalidJson));
        }
        assert_eq!(
            Request::decode(&vec![b' '; MAX_REQUEST_BYTES + 1]),
            Err(ProtocolError::MessageTooLarge)
        );
        assert_eq!(
            Response::decode(&vec![b' '; MAX_RESPONSE_BYTES + 1], &request()),
            Err(ProtocolError::MessageTooLarge)
        );
        assert_eq!(
            Request::decode(br#"{"version":2,"future":true}"#),
            Err(ProtocolError::UnsupportedVersion)
        );
        assert_eq!(
            Response::decode(br#"{"version":2,"future":true}"#, &request()),
            Err(ProtocolError::UnsupportedVersion)
        );
        let deep = format!(
            "{{\"version\":1,\"state\":{}0{}}}",
            "[".repeat(150),
            "]".repeat(150)
        );
        assert!(Request::decode(deep.as_bytes()).is_err());
    }
    #[test]
    fn strict_fields_and_tags() {
        let req = request();
        for pointer in ["", "/state"] {
            let mut value = serde_json::to_value(&req).unwrap();
            value
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unexpected".into(), json!(true));
            assert_eq!(
                Request::decode(&serde_json::to_vec(&value).unwrap()),
                Err(ProtocolError::InvalidJson)
            );
        }
        for pointer in [
            "",
            "/outcome",
            "/outcome/state",
            "/outcome/items/0",
            "/outcome/items/0/view",
            "/outcome/items/0/actions/0",
            "/outcome/items/0/actions/0/effect",
        ] {
            let mut value = serde_json::to_value(response()).unwrap();
            value
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unexpected".into(), json!(true));
            assert_eq!(
                Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
                Err(ProtocolError::InvalidJson)
            );
        }
        for pointer in [
            "/outcome/kind",
            "/outcome/items/0/view/kind",
            "/outcome/items/0/actions/0/effect/kind",
        ] {
            let mut value = serde_json::to_value(response()).unwrap();
            *value.pointer_mut(pointer).unwrap() = json!("Text");
            assert!(Response::decode(&serde_json::to_vec(&value).unwrap(), &req).is_err());
        }
    }
    #[test]
    fn correlation_counts_and_uniqueness() {
        let mut req = request();
        let mut res = response();
        res.generation.push('x');
        assert_eq!(res.encode(&req), Err(ProtocolError::CorrelationMismatch));
        res = response();
        res.request_id.push('x');
        assert_eq!(res.encode(&req), Err(ProtocolError::CorrelationMismatch));
        res = response();
        req.limit = 0;
        req.query.clear();
        assert!(req.encode().is_ok());
        assert_eq!(res.encode(&req), Err(ProtocolError::TooManyResults));
        items(&mut res).clear();
        assert!(res.encode(&req).is_ok());
        req.limit = MAX_RESULTS + 1;
        assert_eq!(req.encode(), Err(ProtocolError::InvalidField("limit")));
        res = response();
        let duplicate = items(&mut res)[0].id.clone();
        items(&mut res)[1].id = duplicate;
        assert_eq!(res.encode(&request()), Err(ProtocolError::DuplicateId("item.id")));
        res = response();
        let duplicate = items(&mut res)[0].actions[0].clone();
        items(&mut res)[0].actions.push(duplicate);
        assert_eq!(
            res.encode(&request()),
            Err(ProtocolError::DuplicateId("action.id"))
        );
    }
    #[test]
    fn state_and_string_bounds_use_encoded_bytes() {
        let mut req = request();
        req.command = "é".repeat(65);
        assert_eq!(req.encode(), Err(ProtocolError::InvalidField("command")));
        req = request();
        req.request_id.push('\n');
        assert_eq!(req.encode(), Err(ProtocolError::InvalidField("request_id")));
        req = request();
        req.query = "x".repeat(MAX_QUERY_BYTES + 1);
        assert!(req.encode().is_err());
        req = request();
        req.state = Some(State {
            version: 0,
            data: json!(null),
        });
        assert_eq!(req.encode(), Err(ProtocolError::InvalidField("state.version")));
        req.state = Some(State {
            version: 1,
            data: json!("x".repeat(MAX_STATE_BYTES - 2)),
        });
        assert!(req.encode().is_ok());
        req.state.as_mut().unwrap().data = json!("x".repeat(MAX_STATE_BYTES - 1));
        assert_eq!(req.encode(), Err(ProtocolError::InvalidField("state.data")));
        req.state.as_mut().unwrap().data = json!("\u{0}".repeat(2000));
        assert_eq!(req.encode(), Err(ProtocolError::InvalidField("state.data")));
    }
    #[test]
    fn escaping_cannot_overrun_output_caps() {
        let mut req = request();
        req.query = "\u{0}".repeat(MAX_QUERY_BYTES);
        req.state = None;
        assert_eq!(req.encode(), Err(ProtocolError::MessageTooLarge));
        let mut res = response();
        items(&mut res)[1].view = View::Detail {
            title: "Large".into(),
            body: "\u{0}".repeat(64 * 1024),
        };
        assert_eq!(res.encode(&request()), Err(ProtocolError::MessageTooLarge));
        let mut writer = CappedWriter {
            bytes: Vec::new(),
            cap: 10,
            exceeded: false,
        };
        assert!(serde_json::to_writer(&mut writer, &"\u{0}".repeat(100)).is_err());
        assert!(writer.exceeded);
        assert!(writer.bytes.len() <= 10);
    }
    #[test]
    fn errors_never_expose_payloads() {
        let secret = "PRIVATE_GUEST_CONTENT";
        let bytes = format!("{{\"version\":1,\"{secret}\":{secret}}}");
        let error = Request::decode(bytes.as_bytes()).unwrap_err();
        assert_eq!(error.to_string(), "invalid JSON structure");
        assert!(!format!("{error:?}").contains(secret));
        let mut req = request();
        req.command = secret.repeat(20);
        assert_eq!(req.encode().unwrap_err().to_string(), "invalid field: command");
    }

    #[test]
    fn decoding_enforces_semantic_bounds_and_correlation() {
        let req = request();
        let mut value = serde_json::to_value(response()).unwrap();
        value["generation"] = json!("stale");
        assert_eq!(
            Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
            Err(ProtocolError::CorrelationMismatch)
        );
        value = serde_json::to_value(response()).unwrap();
        value["outcome"]["items"][0]["actions"] = json!(
            (0..9)
                .map(
                    |i| json!({"id": i.to_string(), "title": "Copy", "effect": {"kind": "copy", "text": "x"}})
                )
                .collect::<Vec<_>>()
        );
        assert_eq!(
            Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
            Err(ProtocolError::InvalidField("actions"))
        );
        for (field, text) in [
            ("title", "x".repeat(513)),
            ("subtitle", "x".repeat(2049)),
            ("title", String::new()),
        ] {
            value = serde_json::to_value(response()).unwrap();
            value["outcome"]["items"][0]["view"][field] = json!(text);
            assert_eq!(
                Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
                Err(ProtocolError::InvalidField(field))
            );
        }
        value = serde_json::to_value(response()).unwrap();
        value["outcome"]["items"][0]["actions"][0]["id"] = json!("copy\u{7f}");
        assert_eq!(
            Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
            Err(ProtocolError::InvalidField("action.id"))
        );
        let bytes = response().encode(&req).unwrap();
        let mut zero = req;
        zero.limit = 0;
        assert_eq!(
            Response::decode(&bytes, &zero),
            Err(ProtocolError::TooManyResults)
        );
    }

    #[test]
    fn actions_only_copy_text_or_open_web_urls() {
        let req = request();
        let with_effect = |effect: serde_json::Value| {
            let mut value = serde_json::to_value(response()).unwrap();
            value["outcome"]["items"][0]["actions"][0]["effect"] = effect;
            Response::decode(&serde_json::to_vec(&value).unwrap(), &req)
        };
        for url in [
            "http://example.com",
            "HTTPS://example.com/path?q=1#top",
            "https://[::1]:8080/",
        ] {
            assert!(
                with_effect(json!({"kind": "open_url", "url": url})).is_ok(),
                "{url}"
            );
        }
        for url in [
            "",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "x-apple.systempreferences:com.apple.preference",
            "https://",
            "https:///path",
            "https://example.com/a b",
            "https://example.com/\n",
            "http//example.com",
        ] {
            assert_eq!(
                with_effect(json!({"kind": "open_url", "url": url})),
                Err(ProtocolError::InvalidField("effect.url")),
                "{url}"
            );
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES));
        assert!(with_effect(json!({"kind": "open_url", "url": long})).is_err());
        assert_eq!(
            with_effect(json!({"kind": "copy", "text": ""})),
            Err(ProtocolError::InvalidField("effect.text"))
        );
        for effect in [
            json!({"kind": "run", "command": "rm"}),
            json!({"kind": "open_path", "path": "/"}),
            json!(null),
        ] {
            assert_eq!(with_effect(effect), Err(ProtocolError::InvalidJson));
        }
        let mut value = serde_json::to_value(response()).unwrap();
        value["outcome"]["items"][0]["actions"][0]
            .as_object_mut()
            .unwrap()
            .remove("effect");
        assert_eq!(
            Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
            Err(ProtocolError::InvalidJson)
        );
    }

    fn view_request() -> Request {
        Request {
            view: Some("stats".into()),
            context: Some(json!({"text": "two words"})),
            ..request()
        }
    }
    fn push() -> Effect {
        Effect::Push {
            view: "stats".into(),
            title: "Statistics".into(),
            context: Some(json!({"text": "two words"})),
        }
    }

    #[test]
    fn requests_outside_a_view_stay_version_one_messages() {
        let bytes = request().encode().unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.get("view").is_none() && value.get("context").is_none());
        let v1 = br#"{"version":1,"request_id":"1","generation":"1","command":"c","query":"q","limit":1,"state":null}"#;
        let decoded = Request::decode(v1).unwrap();
        assert_eq!((decoded.view, decoded.context), (None, None));
    }

    #[test]
    fn view_requests_carry_a_bounded_context() {
        let req = view_request();
        assert_eq!(Request::decode(&req.encode().unwrap()).unwrap(), req);
        let mut orphan = request();
        orphan.context = Some(json!(1));
        assert_eq!(orphan.encode(), Err(ProtocolError::InvalidField("context")));
        let mut bad = view_request();
        bad.view = Some("stats\n".into());
        assert_eq!(bad.encode(), Err(ProtocolError::InvalidField("view")));
        bad.view = Some(String::new());
        assert_eq!(bad.encode(), Err(ProtocolError::InvalidField("view")));
        let nested = |count| (0..count).fold(json!(null), |value, _| json!([value]));
        let mut deep = view_request();
        deep.context = Some(nested(MAX_CONTEXT_DEPTH));
        assert!(deep.encode().is_ok());
        deep.context = Some(nested(MAX_CONTEXT_DEPTH + 1));
        assert_eq!(deep.encode(), Err(ProtocolError::InvalidField("context")));
        let mut large = view_request();
        large.context = Some(json!("x".repeat(MAX_CONTEXT_BYTES - 2)));
        assert!(large.encode().is_ok());
        large.context = Some(json!("x".repeat(MAX_CONTEXT_BYTES - 1)));
        assert_eq!(large.encode(), Err(ProtocolError::InvalidField("context")));
    }

    #[test]
    fn push_effects_name_a_view_with_a_title_and_bounded_context() {
        let req = request();
        let mut res = response();
        items(&mut res)[0].actions[0].effect = push();
        assert_eq!(Response::decode(&res.encode(&req).unwrap(), &req).unwrap(), res);
        let without_context = Effect::Push {
            view: "stats".into(),
            title: "Statistics".into(),
            context: None,
        };
        let encoded = serde_json::to_value(&without_context).unwrap();
        assert!(encoded.get("context").is_none());
        assert_eq!(
            serde_json::from_value::<Effect>(encoded).unwrap(),
            without_context
        );
        for (effect, field) in [
            (
                json!({"kind": "push", "view": "a\u{7}", "title": "T"}),
                "effect.view",
            ),
            (json!({"kind": "push", "view": "a", "title": ""}), "effect.title"),
            (
                json!({"kind": "push", "view": "a", "title": "Two\nLines"}),
                "effect.title",
            ),
            (
                json!({"kind": "push", "view": "a", "title": "T", "context": "x".repeat(MAX_CONTEXT_BYTES)}),
                "effect.context",
            ),
        ] {
            let mut value = serde_json::to_value(response()).unwrap();
            value["outcome"]["items"][0]["actions"][0]["effect"] = effect;
            assert_eq!(
                Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
                Err(ProtocolError::InvalidField(field))
            );
        }
        let mut value = serde_json::to_value(&res).unwrap();
        value["outcome"]["items"][0]["actions"][0]["effect"]["script"] = json!("x");
        assert_eq!(
            Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
            Err(ProtocolError::InvalidJson)
        );
    }

    #[test]
    fn only_a_view_request_may_be_answered_with_a_detail() {
        let req = view_request();
        let detail = Response {
            version: VERSION,
            request_id: req.request_id.clone(),
            generation: req.generation.clone(),
            outcome: Outcome::Detail {
                title: "Statistics".into(),
                body: "2 words\n9 characters".into(),
                actions: vec![PluginAction {
                    id: "deeper".into(),
                    title: "More".into(),
                    effect: push(),
                }],
            },
        };
        assert_eq!(
            Response::decode(&detail.encode(&req).unwrap(), &req).unwrap(),
            detail
        );
        let mut list = response();
        list.request_id = req.request_id.clone();
        assert!(list.encode(&req).is_ok());
        assert_eq!(
            detail.encode(&request()),
            Err(ProtocolError::InvalidField("outcome"))
        );
        let mut value = serde_json::to_value(&detail).unwrap();
        value["outcome"]["body"] = json!("");
        assert_eq!(
            Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
            Err(ProtocolError::InvalidField("body"))
        );
        value = serde_json::to_value(&detail).unwrap();
        value["outcome"]["actions"] = json!(
            (0..9)
                .map(
                    |i| json!({"id": i.to_string(), "title": "Copy", "effect": {"kind": "copy", "text": "x"}})
                )
                .collect::<Vec<_>>()
        );
        assert_eq!(
            Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
            Err(ProtocolError::InvalidField("actions"))
        );
        value = serde_json::to_value(&detail).unwrap();
        value["outcome"]["unexpected"] = json!(true);
        assert_eq!(
            Response::decode(&serde_json::to_vec(&value).unwrap(), &req),
            Err(ProtocolError::InvalidJson)
        );
    }
}
