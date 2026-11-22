//! Experimental runtime-independent wire boundary, not a stable external SDK.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::HashSet, fmt, io};

pub const VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub const MAX_RESULTS: u32 = 500;
pub const MAX_QUERY_BYTES: usize = 4096;
pub const MAX_STATE_BYTES: usize = 8192;

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
}

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
                    if item.actions.len() > 8 {
                        return Err(ProtocolError::InvalidField("actions"));
                    }
                    let mut actions = HashSet::new();
                    for action in &item.actions {
                        identifier(&action.id, "action.id")?;
                        bounded(&action.title, 512, "action.title")?;
                        if !actions.insert(&action.id) {
                            return Err(ProtocolError::DuplicateId("action.id"));
                        }
                    }
                }
                state(saved)
            }
        }
    }
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
    bounded(value, 128, field)?;
    if value.chars().any(char::is_control) {
        return Err(ProtocolError::InvalidField(field));
    }
    Ok(())
}
fn state(value: &Option<State>) -> Result<(), ProtocolError> {
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
    if let Some(value) = value {
        if value.version == 0 {
            return Err(ProtocolError::InvalidField("state.version"));
        }
        if !depth(&value.data, 64) {
            return Err(ProtocolError::InvalidField("state.data"));
        }
        encode_json(&value.data, MAX_STATE_BYTES).map_err(|_| ProtocolError::InvalidField("state.data"))?;
    }
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
                        actions: vec![PluginAction {
                            id: "copy".into(),
                            title: "Copy result".into(),
                        }],
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
        for pointer in ["/outcome/kind", "/outcome/items/0/view/kind"] {
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
                .map(|i| json!({"id": i.to_string(), "title": "Copy"}))
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
}
