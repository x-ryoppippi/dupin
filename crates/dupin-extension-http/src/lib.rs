//! The host side of the extension `http` capability.
//!
//! The broker calls [`HttpService::fetch`] with a guest's [`Request`] and the [`HttpScope`] the
//! user granted that installation. The service never prompts, stores grants or knows which
//! extension is asking: consent, revocation and delivery belong to the broker.

use std::fmt;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

mod address;
mod transport;
mod url;

#[cfg(all(test, target_os = "macos"))]
mod tests;

pub use address::PrivateNetworks;
pub use url::Origin;

use url::{Host, Url};

/// An HTTP method an extension may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
}

impl Method {
    /// Parses a method name, ignoring ASCII case as `fetch` does.
    pub fn parse(name: &str) -> Result<Self, HttpError> {
        Ok(match name.to_ascii_uppercase().as_str() {
            "GET" => Self::Get,
            "HEAD" => Self::Head,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "PATCH" => Self::Patch,
            "DELETE" => Self::Delete,
            "OPTIONS" => Self::Options,
            _ => return Err(HttpError::InvalidRequest(format!("unsupported method `{name}`"))),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
        }
    }

    fn allows_body(self) -> bool {
        !matches!(self, Self::Get | Self::Head)
    }
}

/// One granted origin and the methods allowed against it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRule {
    pub origin: Origin,
    pub methods: Vec<Method>,
}

/// What an installation may reach: the manifest's `capabilities.http` entries the user granted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpScope {
    rules: Vec<HttpRule>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    origin: String,
    methods: Vec<String>,
}

impl HttpScope {
    pub fn new(rules: Vec<HttpRule>) -> Self {
        Self { rules }
    }

    /// Reads the manifest shape, `[{ "origin": "https://…", "methods": ["GET"] }]`.
    pub fn from_manifest(value: &serde_json::Value) -> Result<Self, String> {
        let raw: Vec<RawRule> = serde_json::from_value(value.clone()).map_err(|e| format!("`http`: {e}"))?;
        let mut rules = Vec::with_capacity(raw.len());
        for (index, rule) in raw.into_iter().enumerate() {
            let origin = Origin::parse(&rule.origin).map_err(|e| format!("`http[{index}].origin`: {e}"))?;
            if rule.methods.is_empty() {
                return Err(format!("`http[{index}].methods` must name at least one method"));
            }
            let methods = rule
                .methods
                .iter()
                .map(|name| Method::parse(name))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("`http[{index}].methods`: {e}"))?;
            rules.push(HttpRule { origin, methods });
        }
        Ok(Self { rules })
    }

    pub fn rules(&self) -> &[HttpRule] {
        &self.rules
    }

    fn permits(&self, origin: &Origin, method: Method) -> bool {
        self.rules
            .iter()
            .any(|rule| rule.origin == *origin && rule.methods.contains(&method))
    }
}

/// A guest's request, as the broker decoded it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }
}

/// A completed response. `headers` have lowercase names, without cookies or hop-by-hop fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// The final URL, after redirects.
    pub url: String,
    pub redirected: bool,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The first value of a header, by case-insensitive name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The body as UTF-8 text, without a byte-order mark.
    pub fn text(&self) -> Result<&str, HttpError> {
        let body = self.body.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&self.body);
        std::str::from_utf8(body).map_err(|e| HttpError::InvalidBody(format!("the body is not UTF-8: {e}")))
    }

    /// The body parsed as JSON. Nesting is capped by `serde_json`'s recursion limit.
    pub fn json(&self) -> Result<serde_json::Value, HttpError> {
        serde_json::from_str(self.text()?)
            .map_err(|e| HttpError::InvalidBody(format!("the body is not JSON: {e}")))
    }
}

/// Bounds on one fetch, redirects included. The broker may lower them per call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub total_time: Duration,
    pub connect_time: Duration,
    pub max_redirects: u8,
    pub max_request_headers: usize,
    pub max_request_body_bytes: usize,
    pub max_header_bytes: usize,
    pub max_response_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            total_time: Duration::from_secs(30),
            connect_time: Duration::from_secs(10),
            max_redirects: 5,
            max_request_headers: 64,
            max_request_body_bytes: 1024 * 1024,
            max_header_bytes: 32 * 1024,
            max_response_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HttpConfig {
    pub user_agent: String,
    pub limits: Limits,
    pub private_networks: PrivateNetworks,
    /// The curl executable. Dupin already fetches currency rates and updates through the system
    /// curl, so the capability adds no TLS stack to the binary.
    pub curl: PathBuf,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            user_agent: "Dupin (+https://dupin.app)".to_owned(),
            limits: Limits::default(),
            private_networks: PrivateNetworks::Block,
            curl: PathBuf::from("/usr/bin/curl"),
        }
    }
}

/// Cancels a fetch from another thread. Cloning shares the token.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    InvalidRequest(String),
    OutOfScope { origin: String, method: Method },
    RedirectOutOfScope { origin: String, method: Method },
    InvalidRedirect(String),
    TooManyRedirects(u8),
    BlockedAddress { host: String, reason: &'static str },
    Dns(String),
    Connect(String),
    Tls(String),
    Network(String),
    Timeout,
    Cancelled,
    HeadersTooLarge(usize),
    ResponseTooLarge(usize),
    InvalidResponse(String),
    InvalidBody(String),
}

impl HttpError {
    /// A stable code for the guest's error object.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid-request",
            Self::OutOfScope { .. } => "out-of-scope",
            Self::RedirectOutOfScope { .. } => "redirect-out-of-scope",
            Self::InvalidRedirect(_) => "invalid-redirect",
            Self::TooManyRedirects(_) => "too-many-redirects",
            Self::BlockedAddress { .. } => "blocked-address",
            Self::Dns(_) => "dns",
            Self::Connect(_) => "connect",
            Self::Tls(_) => "tls",
            Self::Network(_) => "network",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::HeadersTooLarge(_) => "headers-too-large",
            Self::ResponseTooLarge(_) => "response-too-large",
            Self::InvalidResponse(_) => "invalid-response",
            Self::InvalidBody(_) => "invalid-body",
        }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(why)
            | Self::InvalidRedirect(why)
            | Self::InvalidResponse(why)
            | Self::InvalidBody(why) => f.write_str(why),
            Self::OutOfScope { origin, method } => {
                write!(
                    f,
                    "{} {origin} is not in the extension's granted http scope",
                    method.as_str()
                )
            }
            Self::RedirectOutOfScope { origin, method } => write!(
                f,
                "a redirect led to {} {origin}, which is not in the extension's granted http scope",
                method.as_str()
            ),
            Self::TooManyRedirects(max) => write!(f, "more than {max} redirects"),
            Self::BlockedAddress { host, reason } => {
                write!(f, "{host} resolves to {reason}, which extensions can't reach")
            }
            Self::Dns(why) => write!(f, "couldn't resolve the host: {why}"),
            Self::Connect(why) => write!(f, "couldn't connect: {why}"),
            Self::Tls(why) => write!(f, "TLS failed: {why}"),
            Self::Network(why) => write!(f, "the request failed: {why}"),
            Self::Timeout => f.write_str("the request took too long"),
            Self::Cancelled => f.write_str("the request was cancelled"),
            Self::HeadersTooLarge(max) => write!(f, "the response headers exceed {max} bytes"),
            Self::ResponseTooLarge(max) => write!(f, "the response body exceeds {max} bytes"),
        }
    }
}

impl std::error::Error for HttpError {}

type Resolve = dyn Fn(&str, u16) -> std::io::Result<Vec<IpAddr>> + Send + Sync;

pub struct HttpService {
    config: HttpConfig,
    resolve: Arc<Resolve>,
    loopback: bool,
}

/// Request headers a guest can't set: the transport owns framing, the connection and the host,
/// and servers attribute extension traffic to Dupin by its user agent.
const FORBIDDEN_REQUEST_HEADERS: &[&str] = &[
    "connection",
    "content-length",
    "expect",
    "host",
    "keep-alive",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "user-agent",
];

/// Response headers a guest never sees: cookies would be ambient state, the rest is transport.
const HIDDEN_RESPONSE_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-connection",
    "set-cookie",
    "set-cookie2",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

impl HttpService {
    pub fn new(config: HttpConfig) -> Self {
        Self {
            config,
            resolve: Arc::new(|host, port| Ok((host, port).to_socket_addrs()?.map(|a| a.ip()).collect())),
            loopback: false,
        }
    }

    /// Lets tests reach a local server through a fixed resolver. Never compiled into Dupin.
    #[cfg(all(test, target_os = "macos"))]
    fn for_tests(config: HttpConfig, resolve: Arc<Resolve>) -> Self {
        Self {
            config,
            resolve,
            loopback: true,
        }
    }

    /// Fetches `request` within `scope`, following redirects that stay inside it.
    ///
    /// Every hop is checked against the scope and its resolved addresses against the address
    /// policy, and curl connects only to the addresses that were checked. Blocks the calling
    /// thread until the response is complete, a limit is hit or `cancel` fires.
    pub fn fetch(
        &self,
        request: Request,
        scope: &HttpScope,
        cancel: &CancelToken,
    ) -> Result<Response, HttpError> {
        let limits = &self.config.limits;
        let deadline = Instant::now() + limits.total_time;
        let Request {
            mut method,
            url,
            mut headers,
            mut body,
        } = request;
        let mut url = self.parse_url(&url).map_err(HttpError::InvalidRequest)?;
        self.validate_headers(&headers)?;
        if let Some(bytes) = &body {
            if !method.allows_body() {
                return Err(HttpError::InvalidRequest(format!(
                    "a {} request can't have a body",
                    method.as_str()
                )));
            }
            if bytes.len() > limits.max_request_body_bytes {
                return Err(HttpError::InvalidRequest(format!(
                    "the request body exceeds {} bytes",
                    limits.max_request_body_bytes
                )));
            }
        } else if matches!(method, Method::Post | Method::Put | Method::Patch) {
            body = Some(Vec::new());
        }
        if !scope.permits(&url.origin, method) {
            return Err(HttpError::OutOfScope {
                origin: url.origin.to_string(),
                method,
            });
        }

        let mut redirects = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(HttpError::Cancelled);
            }
            let addresses = self.resolve_checked(&url, deadline, cancel)?;
            let raw = transport::Exchange {
                curl: &self.config.curl,
                user_agent: &self.config.user_agent,
                limits,
                deadline,
                cancel,
                url: &url,
                addresses: &addresses,
                method,
                headers: &headers,
                body: body.as_deref(),
            }
            .run()?;

            let location = raw
                .headers
                .iter()
                .find(|(name, _)| name == "location")
                .map(|(_, value)| value);
            let Some(location) = location.filter(|_| matches!(raw.status, 301 | 302 | 303 | 307 | 308))
            else {
                return Ok(Response {
                    status: raw.status,
                    url: url.to_string(),
                    redirected: redirects > 0,
                    headers: raw
                        .headers
                        .into_iter()
                        .filter(|(name, _)| !HIDDEN_RESPONSE_HEADERS.contains(&name.as_str()))
                        .collect(),
                    body: raw.body,
                });
            };
            if redirects == limits.max_redirects {
                return Err(HttpError::TooManyRedirects(limits.max_redirects));
            }
            redirects += 1;
            let next = url.join(location).map_err(HttpError::InvalidRedirect)?;
            // Fetch's rules: 303 becomes GET, and so does a POST answered with 301 or 302.
            if (raw.status == 303 && method != Method::Head)
                || (matches!(raw.status, 301 | 302) && method == Method::Post)
            {
                method = Method::Get;
                body = None;
                headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-type"));
            }
            if next.origin != url.origin {
                headers.retain(|(name, _)| {
                    !name.eq_ignore_ascii_case("authorization") && !name.eq_ignore_ascii_case("cookie")
                });
            }
            if !scope.permits(&next.origin, method) {
                return Err(HttpError::RedirectOutOfScope {
                    origin: next.origin.to_string(),
                    method,
                });
            }
            url = next;
        }
    }

    fn parse_url(&self, text: &str) -> Result<Url, String> {
        Url::parse(text, self.loopback)
    }

    fn validate_headers(&self, headers: &[(String, String)]) -> Result<(), HttpError> {
        let invalid = |why: String| Err(HttpError::InvalidRequest(why));
        if headers.len() > self.config.limits.max_request_headers {
            return invalid(format!(
                "more than {} request headers",
                self.config.limits.max_request_headers
            ));
        }
        for (name, value) in headers {
            let token = !name.is_empty()
                && name.len() <= 256
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
            if !token {
                return invalid(format!("`{name}` is not a valid header name"));
            }
            let lower = name.to_ascii_lowercase();
            if FORBIDDEN_REQUEST_HEADERS.contains(&lower.as_str()) || lower.starts_with("proxy-") {
                return invalid(format!("extensions can't set the `{name}` header"));
            }
            if value.len() > 8192 || value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
                return invalid(format!(
                    "the `{name}` header value is too long or contains a line break"
                ));
            }
        }
        Ok(())
    }

    fn resolve_checked(
        &self,
        url: &Url,
        deadline: Instant,
        cancel: &CancelToken,
    ) -> Result<Vec<IpAddr>, HttpError> {
        let host = url.origin.host.to_string();
        let addresses = match &url.origin.host {
            Host::Ip(ip) => vec![*ip],
            Host::Domain(name) => self.resolve_with_deadline(name, url.origin.port, deadline, cancel)?,
        };
        if addresses.is_empty() {
            return Err(HttpError::Dns(format!("{host} has no addresses")));
        }
        // Every record must pass: curl may use any of them, and a mixed answer is a rebinding tell.
        for ip in &addresses {
            let class = address::classify(*ip);
            if !address::allowed(class, self.config.private_networks, self.loopback) {
                return Err(HttpError::BlockedAddress {
                    host,
                    reason: class.describe(),
                });
            }
        }
        Ok(addresses)
    }

    fn resolve_with_deadline(
        &self,
        name: &str,
        port: u16,
        deadline: Instant,
        cancel: &CancelToken,
    ) -> Result<Vec<IpAddr>, HttpError> {
        let (sender, receiver) = mpsc::channel();
        let resolve = Arc::clone(&self.resolve);
        let owned = name.to_owned();
        // getaddrinfo can't be interrupted; a timed-out lookup finishes on its own thread.
        thread::spawn(move || {
            let _ = sender.send(resolve(&owned, port));
        });
        loop {
            if cancel.is_cancelled() {
                return Err(HttpError::Cancelled);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(HttpError::Timeout);
            }
            match receiver.recv_timeout(Duration::from_millis(20).min(deadline - now)) {
                Ok(result) => return result.map_err(|e| HttpError::Dns(format!("{name}: {e}"))),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(HttpError::Dns(format!("{name}: the resolver stopped")));
                }
            }
        }
    }
}
