use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::address::{Class, allowed, classify};
use crate::url::Url;

#[derive(Clone, Debug)]
struct Seen {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

type Handler = dyn Fn(&Seen, &mut TcpStream) + Send + Sync;

/// A plain-HTTP server on loopback that records every request it answers.
struct Server {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    fn start(handler: impl Fn(&Seen, &mut TcpStream) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let log = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let handler = Arc::clone(&handler);
                let log = Arc::clone(&log);
                thread::spawn(move || {
                    let Some(request) = read_request(&mut stream) else {
                        return;
                    };
                    log.lock().unwrap().push(request.clone());
                    handler(&request, &mut stream);
                });
            }
        });
        Self { port, seen }
    }

    fn respond(raw: &'static str) -> Self {
        Self::start(move |_, stream| {
            let _ = stream.write_all(raw.as_bytes());
        })
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.push((name.to_owned(), value.trim().to_owned()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Seen {
        request_line: request_line.trim_end().to_owned(),
        headers,
        body,
    })
}

fn loopback_resolver() -> Arc<Resolve> {
    Arc::new(|host, _| match host {
        "rebind.test" | "localhost" => Ok(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]),
        "mixed.test" => Ok(vec![
            IpAddr::V4(Ipv4Addr::new(93, 184, 215, 14)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        ]),
        "metadata.test" => Ok(vec![IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))]),
        _ => Err(std::io::Error::other("unknown test host")),
    })
}

fn service(limits: Limits) -> HttpService {
    HttpService::for_tests(
        HttpConfig {
            user_agent: "Dupin/test (+https://dupin.app)".to_owned(),
            limits,
            ..HttpConfig::default()
        },
        loopback_resolver(),
    )
}

fn scope(rules: &[(&str, &[Method])]) -> HttpScope {
    HttpScope::new(
        rules
            .iter()
            .map(|(origin, methods)| HttpRule {
                origin: Origin::parse_allowing_http(origin).unwrap(),
                methods: methods.to_vec(),
            })
            .collect(),
    )
}

fn fetch(service: &HttpService, request: Request, scope: &HttpScope) -> Result<Response, HttpError> {
    service.fetch(request, scope, &CancelToken::new())
}

const OK_JSON: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: id=1\r\nX-Rate: 5\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"a\":[1,2]}";

#[test]
fn an_in_scope_get_returns_status_filtered_headers_and_body() {
    let server = Server::respond(OK_JSON);
    let scope = scope(&[(&server.origin(), &[Method::Get])]);
    let response = fetch(
        &service(Limits::default()),
        Request::get(format!("{}/data?x=1", server.origin())),
        &scope,
    )
    .unwrap();

    assert_eq!(response.status, 200);
    assert!(response.ok());
    assert!(!response.redirected);
    assert_eq!(response.url, format!("{}/data?x=1", server.origin()));
    assert_eq!(response.header("x-rate"), Some("5"));
    assert_eq!(response.header("set-cookie"), None);
    assert_eq!(response.header("connection"), None);
    assert_eq!(response.text().unwrap(), "{\"a\":[1,2]}");
    assert_eq!(response.json().unwrap(), serde_json::json!({ "a": [1, 2] }));

    let seen = &server.requests()[0];
    assert_eq!(seen.request_line, "GET /data?x=1 HTTP/1.1");
    assert_eq!(seen.header("user-agent"), Some("Dupin/test (+https://dupin.app)"));
    assert_eq!(seen.header("cookie"), None);
    assert_eq!(seen.header("authorization"), None);
}

#[test]
fn a_post_sends_its_body_and_headers_without_a_default_content_type() {
    let server = Server::respond("HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
    let scope = scope(&[(&server.origin(), &[Method::Post])]);
    let request = Request {
        method: Method::Post,
        url: format!("{}/items", server.origin()),
        headers: vec![
            ("X-Token".into(), "abc".into()),
            ("X-Empty".into(), String::new()),
        ],
        body: Some(b"raw bytes".to_vec()),
    };
    let response = fetch(&service(Limits::default()), request, &scope).unwrap();

    assert_eq!(response.status, 204);
    let seen = &server.requests()[0];
    assert_eq!(seen.request_line, "POST /items HTTP/1.1");
    assert_eq!(seen.body, b"raw bytes");
    assert_eq!(seen.header("x-token"), Some("abc"));
    assert_eq!(seen.header("x-empty"), Some(""));
    assert_eq!(seen.header("content-type"), None);
    assert_eq!(seen.header("expect"), None);
}

#[test]
fn non_success_statuses_are_responses_not_errors() {
    let server =
        Server::respond("HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
    let scope = scope(&[(&server.origin(), &[Method::Get])]);
    let response = fetch(&service(Limits::default()), Request::get(server.origin()), &scope).unwrap();
    assert_eq!(response.status, 404);
    assert!(!response.ok());
    assert_eq!(response.text().unwrap(), "nope");
}

#[test]
fn a_method_or_origin_outside_the_scope_is_refused_before_connecting() {
    let server = Server::respond(OK_JSON);
    let service = service(Limits::default());
    let get_only = scope(&[(&server.origin(), &[Method::Get])]);

    let post = Request {
        method: Method::Post,
        ..Request::get(server.origin())
    };
    assert!(matches!(
        fetch(&service, post, &get_only),
        Err(HttpError::OutOfScope {
            method: Method::Post,
            ..
        })
    ));

    let other_port = scope(&[(
        &format!("http://127.0.0.1:{}", server.port.wrapping_add(1)),
        &[Method::Get],
    )]);
    assert_eq!(
        fetch(&service, Request::get(server.origin()), &other_port)
            .unwrap_err()
            .code(),
        "out-of-scope"
    );

    let by_name = scope(&[(&format!("http://localhost:{}", server.port), &[Method::Get])]);
    assert_eq!(
        fetch(&service, Request::get(server.origin()), &by_name)
            .unwrap_err()
            .code(),
        "out-of-scope"
    );

    assert!(server.requests().is_empty());
}

fn redirect_to(location: String, status: u16) -> impl Fn(&Seen, &mut TcpStream) + Send + Sync + 'static {
    move |_, stream| {
        let _ = write!(
            stream,
            "HTTP/1.1 {status} Moved\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
    }
}

#[test]
fn a_redirect_out_of_the_scope_is_not_followed() {
    let target = Server::respond(OK_JSON);
    let start = Server::start(redirect_to(format!("{}/secret", target.origin()), 302));
    let scope = scope(&[(&start.origin(), &[Method::Get])]);

    let error = fetch(&service(Limits::default()), Request::get(start.origin()), &scope).unwrap_err();
    assert!(matches!(&error, HttpError::RedirectOutOfScope { origin, .. } if *origin == target.origin()));
    assert_eq!(start.requests().len(), 1);
    assert!(target.requests().is_empty());
}

#[test]
fn a_redirect_into_a_scheme_other_than_the_scope_s_is_refused() {
    let start = Server::start(redirect_to("file:///etc/passwd".to_owned(), 301));
    let scope = scope(&[(&start.origin(), &[Method::Get])]);
    let error = fetch(&service(Limits::default()), Request::get(start.origin()), &scope).unwrap_err();
    assert_eq!(error.code(), "invalid-redirect");
}

#[test]
fn in_scope_redirects_are_followed_with_fetch_semantics() {
    let target = Server::respond(OK_JSON);
    let start = Server::start(redirect_to(format!("{}/landing", target.origin()), 303));
    let scope = scope(&[
        (&start.origin(), &[Method::Post]),
        (&target.origin(), &[Method::Get]),
    ]);
    let request = Request {
        method: Method::Post,
        url: start.origin(),
        headers: vec![
            ("Authorization".into(), "Bearer guest-token".into()),
            ("Content-Type".into(), "text/plain".into()),
        ],
        body: Some(b"hello".to_vec()),
    };
    let response = fetch(&service(Limits::default()), request, &scope).unwrap();

    assert!(response.redirected);
    assert_eq!(response.url, format!("{}/landing", target.origin()));
    let seen = &target.requests()[0];
    assert_eq!(seen.request_line, "GET /landing HTTP/1.1");
    assert!(seen.body.is_empty());
    assert_eq!(
        seen.header("authorization"),
        None,
        "a cross-origin redirect drops credentials"
    );
    assert_eq!(seen.header("content-type"), None);
}

#[test]
fn a_redirect_that_keeps_the_method_needs_that_method_in_scope() {
    let target = Server::respond(OK_JSON);
    let start = Server::start(redirect_to(format!("{}/", target.origin()), 307));
    let scope = scope(&[
        (&start.origin(), &[Method::Post]),
        (&target.origin(), &[Method::Get]),
    ]);
    let request = Request {
        method: Method::Post,
        ..Request::get(start.origin())
    };
    let error = fetch(&service(Limits::default()), request, &scope).unwrap_err();
    assert!(matches!(
        error,
        HttpError::RedirectOutOfScope {
            method: Method::Post,
            ..
        }
    ));
    assert!(target.requests().is_empty());
}

#[test]
fn redirect_loops_stop_at_the_limit() {
    let server = Server::start(redirect_to("/again".to_owned(), 302));
    let scope = scope(&[(&server.origin(), &[Method::Get])]);
    let limits = Limits {
        max_redirects: 3,
        ..Limits::default()
    };
    let error = fetch(&service(limits), Request::get(server.origin()), &scope).unwrap_err();
    assert_eq!(error, HttpError::TooManyRedirects(3));
    assert_eq!(server.requests().len(), 4);
}

#[test]
fn loopback_is_blocked_without_the_test_override() {
    let server = Server::respond(OK_JSON);
    let production = HttpService::new(HttpConfig::default());
    for url in ["https://127.0.0.1/", "https://[::1]/", "https://localhost/"] {
        let origin = Url::parse(url, false).unwrap().origin;
        let scope = HttpScope::new(vec![HttpRule {
            origin,
            methods: vec![Method::Get],
        }]);
        let error = fetch(&production, Request::get(url), &scope).unwrap_err();
        assert!(
            matches!(
                error,
                HttpError::BlockedAddress {
                    reason: "a loopback address",
                    ..
                }
            ),
            "{url}: {error}"
        );
    }
    assert!(server.requests().is_empty());
}

#[test]
fn private_link_local_and_metadata_addresses_are_blocked_even_in_tests() {
    let service = service(Limits::default());
    for host in [
        "10.0.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "[fd00::1]",
        "[::ffff:10.0.0.1]",
        "metadata.test",
        "mixed.test",
    ] {
        let url = format!("http://{host}/");
        let scope = scope(&[(&url, &[Method::Get])]);
        let error = fetch(&service, Request::get(&url), &scope).unwrap_err();
        assert_eq!(error.code(), "blocked-address", "{host}: {error}");
    }
}

#[test]
fn a_redirect_to_a_private_address_is_blocked() {
    let start = Server::start(redirect_to(
        "http://169.254.169.254/latest/meta-data/".to_owned(),
        302,
    ));
    let scope = scope(&[
        (&start.origin(), &[Method::Get]),
        ("http://169.254.169.254", &[Method::Get]),
    ]);
    let error = fetch(&service(Limits::default()), Request::get(start.origin()), &scope).unwrap_err();
    assert!(matches!(
        error,
        HttpError::BlockedAddress {
            reason: "a link-local address",
            ..
        }
    ));
}

#[test]
fn curl_connects_to_the_address_that_was_checked() {
    // `rebind.test` resolves only through the service's resolver. If curl looked the name up
    // again, it would fail; reaching the server proves the checked address was pinned.
    let server = Server::respond(OK_JSON);
    let origin = format!("http://rebind.test:{}", server.port);
    let scope = scope(&[(&origin, &[Method::Get])]);
    let response = fetch(
        &service(Limits::default()),
        Request::get(format!("{origin}/x")),
        &scope,
    )
    .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        server.requests()[0].header("host"),
        Some(&*format!("rebind.test:{}", server.port))
    );
}

fn large_body(length: usize, chunked: bool) -> impl Fn(&Seen, &mut TcpStream) + Send + Sync + 'static {
    move |_, stream| {
        let header = if chunked {
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_owned()
        } else {
            format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
        };
        let _ = stream.write_all(header.as_bytes());
        let block = vec![b'a'; 8192];
        let mut sent = 0;
        while sent < length {
            let n = block.len().min(length - sent);
            let result = if chunked {
                write!(stream, "{n:x}\r\n")
                    .and_then(|()| stream.write_all(&block[..n]))
                    .and_then(|()| stream.write_all(b"\r\n"))
            } else {
                stream.write_all(&block[..n])
            };
            if result.is_err() {
                return;
            }
            sent += n;
        }
        if chunked {
            let _ = stream.write_all(b"0\r\n\r\n");
        }
    }
}

#[test]
fn bodies_over_the_limit_are_refused_with_or_without_a_length() {
    let limits = Limits {
        max_response_bytes: 64 * 1024,
        ..Limits::default()
    };
    for chunked in [false, true] {
        let server = Server::start(large_body(1024 * 1024, chunked));
        let scope = scope(&[(&server.origin(), &[Method::Get])]);
        let error = fetch(&service(limits.clone()), Request::get(server.origin()), &scope).unwrap_err();
        assert_eq!(
            error,
            HttpError::ResponseTooLarge(64 * 1024),
            "chunked: {chunked}"
        );
    }

    let server = Server::start(large_body(64 * 1024, true));
    let scope = scope(&[(&server.origin(), &[Method::Get])]);
    let response = fetch(&service(limits), Request::get(server.origin()), &scope).unwrap();
    assert_eq!(response.body.len(), 64 * 1024);
}

#[test]
fn headers_over_the_limit_are_refused() {
    let server = Server::start(|_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n");
        for i in 0..2000 {
            if write!(stream, "X-Filler-{i}: {}\r\n", "f".repeat(40)).is_err() {
                return;
            }
        }
        let _ = stream.write_all(b"Content-Length: 0\r\n\r\n");
    });
    let scope = scope(&[(&server.origin(), &[Method::Get])]);
    let limits = Limits {
        max_header_bytes: 16 * 1024,
        ..Limits::default()
    };
    let error = fetch(&service(limits), Request::get(server.origin()), &scope).unwrap_err();
    assert_eq!(error, HttpError::HeadersTooLarge(16 * 1024));
}

fn stall(for_: Duration) -> impl Fn(&Seen, &mut TcpStream) + Send + Sync + 'static {
    move |_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nhalf");
        thread::sleep(for_);
    }
}

#[test]
fn a_slow_response_times_out_within_the_total_time() {
    let server = Server::start(stall(Duration::from_secs(10)));
    let scope = scope(&[(&server.origin(), &[Method::Get])]);
    let limits = Limits {
        total_time: Duration::from_millis(500),
        ..Limits::default()
    };
    let started = Instant::now();
    let error = fetch(&service(limits), Request::get(server.origin()), &scope).unwrap_err();
    assert_eq!(error, HttpError::Timeout);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn cancelling_stops_a_fetch_in_flight() {
    let server = Server::start(stall(Duration::from_secs(10)));
    let scope = scope(&[(&server.origin(), &[Method::Get])]);
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        trigger.cancel();
    });
    let started = Instant::now();
    let error = service(Limits::default())
        .fetch(Request::get(server.origin()), &scope, &cancel)
        .unwrap_err();
    assert_eq!(error, HttpError::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn an_already_cancelled_token_never_connects() {
    let server = Server::respond(OK_JSON);
    let scope = scope(&[(&server.origin(), &[Method::Get])]);
    let cancel = CancelToken::new();
    cancel.cancel();
    let error = service(Limits::default())
        .fetch(Request::get(server.origin()), &scope, &cancel)
        .unwrap_err();
    assert_eq!(error, HttpError::Cancelled);
    assert!(server.requests().is_empty());
}

#[test]
fn request_headers_and_bodies_are_validated() {
    let service = service(Limits::default());
    let scope = scope(&[("http://127.0.0.1:9", &[Method::Get, Method::Post])]);
    for (name, value) in [
        ("X-Ok", "a\r\nInjected: 1"),
        ("@/etc/passwd", "x"),
        ("Bad Name", "x"),
        ("Host", "evil.example"),
        ("Content-Length", "1"),
        ("Proxy-Authorization", "x"),
        ("Transfer-Encoding", "chunked"),
        ("User-Agent", "Mozilla/5.0"),
    ] {
        let request = Request {
            headers: vec![(name.into(), value.into())],
            ..Request::get("http://127.0.0.1:9/")
        };
        assert_eq!(
            fetch(&service, request, &scope).unwrap_err().code(),
            "invalid-request",
            "{name}"
        );
    }
    let get_with_body = Request {
        body: Some(b"x".to_vec()),
        ..Request::get("http://127.0.0.1:9/")
    };
    assert_eq!(
        fetch(&service, get_with_body, &scope).unwrap_err().code(),
        "invalid-request"
    );
    let too_big = Request {
        method: Method::Post,
        body: Some(vec![0; 2 * 1024 * 1024]),
        ..Request::get("http://127.0.0.1:9/")
    };
    assert_eq!(
        fetch(&service, too_big, &scope).unwrap_err().code(),
        "invalid-request"
    );
}

#[test]
fn urls_that_parsers_could_disagree_on_are_rejected() {
    for url in [
        "http://example.com/",
        "ftp://example.com/",
        "https://user:pass@example.com/",
        "https://example.com@evil.test/",
        "https://example.com\\@evil.test/",
        "https://exa mple.com/",
        "https://exämple.com/",
        "https://127.1/",
        "https://0x7f.0.0.1/",
        "https://2130706433/",
        "https://example.com./",
        "https://-bad.example/",
        "https://example.com:0/",
        "https://example.com:99999/",
        "https://[::1/",
        "https:example.com",
    ] {
        assert!(Url::parse(url, false).is_err(), "{url}");
    }
    let url = Url::parse("HTTPS://API.Example.COM:443?q=1#frag", false).unwrap();
    assert_eq!(url.to_string(), "https://api.example.com/?q=1");
    assert_eq!(
        Url::parse("https://[2001:4860::8888]:8443/a", false)
            .unwrap()
            .to_string(),
        "https://[2001:4860::8888]:8443/a"
    );
}

#[test]
fn redirect_locations_resolve_against_the_current_url() {
    let base = Url::parse("https://a.example/dir/page?x=1", false).unwrap();
    let join = |reference: &str| base.join(reference).map(|url| url.to_string());
    assert_eq!(join("https://b.example/x").unwrap(), "https://b.example/x");
    assert_eq!(join("//c.example/y").unwrap(), "https://c.example/y");
    assert_eq!(join("/root").unwrap(), "https://a.example/root");
    assert_eq!(join("next").unwrap(), "https://a.example/dir/next");
    assert_eq!(join("?y=2").unwrap(), "https://a.example/dir/page?y=2");
    assert!(join("http://a.example/").is_err());
    assert!(join("javascript:alert(1)").is_err());
}

#[test]
fn the_manifest_shape_parses_into_a_scope() {
    let scope = HttpScope::from_manifest(&serde_json::json!([
        { "origin": "https://api.github.com", "methods": ["GET", "head"] }
    ]))
    .unwrap();
    assert_eq!(scope.rules()[0].origin.to_string(), "https://api.github.com");
    assert_eq!(scope.rules()[0].methods, [Method::Get, Method::Head]);

    for bad in [
        serde_json::json!([{ "origin": "http://api.github.com", "methods": ["GET"] }]),
        serde_json::json!([{ "origin": "https://api.github.com/repos", "methods": ["GET"] }]),
        serde_json::json!([{ "origin": "https://api.github.com", "methods": [] }]),
        serde_json::json!([{ "origin": "https://api.github.com", "methods": ["CONNECT"] }]),
        serde_json::json!([{ "origin": "https://api.github.com", "methods": ["GET"], "cookies": true }]),
    ] {
        assert!(HttpScope::from_manifest(&bad).is_err(), "{bad}");
    }
}

#[test]
fn addresses_are_classified_conservatively() {
    let v4 = |a, b, c, d| classify(IpAddr::V4(Ipv4Addr::new(a, b, c, d)));
    let v6 = |text: &str| classify(IpAddr::V6(text.parse::<Ipv6Addr>().unwrap()));
    assert_eq!(v4(93, 184, 215, 14), Class::Public);
    assert_eq!(v4(127, 0, 0, 2), Class::Loopback);
    assert_eq!(v4(172, 20, 0, 1), Class::Private);
    assert_eq!(v4(100, 64, 0, 1), Class::Private);
    assert_eq!(v4(169, 254, 169, 254), Class::LinkLocal);
    assert_eq!(v4(239, 255, 255, 250), Class::Multicast);
    assert_eq!(v4(0, 0, 0, 0), Class::Reserved);
    assert_eq!(v4(255, 255, 255, 255), Class::Reserved);
    assert_eq!(v4(192, 0, 2, 1), Class::Reserved);
    assert_eq!(v6("2606:4700::1111"), Class::Public);
    assert_eq!(v6("::"), Class::Reserved);
    assert_eq!(v6("::1"), Class::Loopback);
    assert_eq!(v6("::ffff:127.0.0.1"), Class::Loopback);
    assert_eq!(v6("64:ff9b::a00:1"), Class::Private);
    assert_eq!(v6("2002:a9fe:a9fe::"), Class::LinkLocal);
    assert_eq!(v6("2001::1"), Class::Reserved);
    assert_eq!(v6("2001:db8::1"), Class::Reserved);
    assert_eq!(v6("fe80::1"), Class::LinkLocal);
    assert_eq!(v6("ff02::1"), Class::Multicast);

    assert!(!allowed(Class::Private, PrivateNetworks::Block, false));
    assert!(allowed(Class::Private, PrivateNetworks::Allow, false));
    assert!(!allowed(Class::Loopback, PrivateNetworks::Allow, false));
    assert!(!allowed(Class::LinkLocal, PrivateNetworks::Allow, true));
}

#[test]
fn text_and_json_reject_bodies_they_cannot_read() {
    let response = |body: &[u8]| Response {
        status: 200,
        url: String::new(),
        redirected: false,
        headers: Vec::new(),
        body: body.to_vec(),
    };
    assert_eq!(response(b"\xef\xbb\xbfhi").text().unwrap(), "hi");
    assert_eq!(response(b"\xff").text().unwrap_err().code(), "invalid-body");
    assert_eq!(response(b"{").json().unwrap_err().code(), "invalid-body");
    let deep = "[".repeat(1000) + &"]".repeat(1000);
    assert_eq!(
        response(deep.as_bytes()).json().unwrap_err().code(),
        "invalid-body"
    );
}

#[test]
fn a_head_reports_the_headers_of_a_resource_larger_than_the_body_limit() {
    let server = Server::respond(
        "HTTP/1.1 200 OK\r\nContent-Length: 100000000\r\nContent-Type: video/mp4\r\nConnection: close\r\n\r\n",
    );
    let scope = scope(&[(&server.origin(), &[Method::Head])]);
    let request = Request {
        method: Method::Head,
        ..Request::get(server.origin())
    };
    let response = fetch(&service(Limits::default()), request, &scope).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-length"), Some("100000000"));
    assert!(response.body.is_empty());
    assert_eq!(server.requests()[0].request_line, "HEAD / HTTP/1.1");
}

#[test]
fn chunked_trailers_stay_out_of_the_body_and_the_headers() {
    let server = Server::respond(
        "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nTrailer: Set-Cookie, Server-Timing\r\nConnection: close\r\n\r\n7\r\n{\"a\":1}\r\n0\r\nSet-Cookie: sid=secret\r\nServer-Timing: db;dur=5\r\n\r\n",
    );
    let scope = scope(&[(&server.origin(), &[Method::Get])]);
    let response = fetch(&service(Limits::default()), Request::get(server.origin()), &scope).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"{\"a\":1}");
    assert_eq!(response.json().unwrap(), serde_json::json!({ "a": 1 }));
    assert_eq!(response.header("set-cookie"), None);
    assert_eq!(response.header("server-timing"), None);
}

#[test]
fn request_headers_and_the_url_stay_out_of_curl_s_arguments() {
    use std::os::unix::fs::PermissionsExt;

    let directory = std::env::temp_dir().join(format!(
        "dupin-http-argv-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let curl = directory.join("curl");
    std::fs::write(
        &curl,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\n/bin/cat /dev/fd/3 > \"$0.config\"\nexit 7\n",
    )
    .unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let service = HttpService::for_tests(
        HttpConfig {
            curl,
            ..HttpConfig::default()
        },
        loopback_resolver(),
    );
    let scope = scope(&[("http://127.0.0.1:9", &[Method::Get])]);
    let request = Request {
        headers: vec![
            ("Authorization".into(), "Bearer secret-token".into()),
            ("X-Quoted".into(), "a \"b\" \\c".into()),
        ],
        ..Request::get("http://127.0.0.1:9/x?key=query-secret")
    };

    let error = fetch(&service, request, &scope).unwrap_err();
    let arguments = std::fs::read_to_string(directory.join("curl.args")).unwrap();
    let config = std::fs::read_to_string(directory.join("curl.config")).unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    assert_eq!(error.code(), "connect");
    assert_eq!(arguments, "--disable\n--config\n/dev/fd/3\n");
    assert!(
        config.contains("header = \"Authorization: Bearer secret-token\"\n"),
        "{config}"
    );
    assert!(
        config.contains("header = \"X-Quoted: a \\\"b\\\" \\\\c\"\n"),
        "{config}"
    );
    assert!(
        config.contains("url = \"http://127.0.0.1:9/x?key=query-secret\"\n"),
        "{config}"
    );
}

#[test]
fn curl_starts_without_forking_the_host() {
    use std::sync::Once;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // On macOS a fork() child of a multithreaded process runs libSystem's atfork handlers, which
    // abort it with SIGKILL when another thread held a lazy-initialization gate at fork time.
    static FORKS: AtomicUsize = AtomicUsize::new(0);
    static COUNT_FORKS: Once = Once::new();
    extern "C" fn count_fork() {
        FORKS.fetch_add(1, Ordering::SeqCst);
    }
    COUNT_FORKS.call_once(|| {
        // SAFETY: the handler only touches an atomic, which is safe in a fork prepare handler.
        assert_eq!(unsafe { libc::pthread_atfork(Some(count_fork), None, None) }, 0);
    });

    let server = Server::respond(OK_JSON);
    let scope = scope(&[(&server.origin(), &[Method::Get, Method::Post])]);
    let service = service(Limits::default());
    let before = FORKS.load(Ordering::SeqCst);
    fetch(&service, Request::get(server.origin()), &scope).unwrap();
    let post = Request {
        method: Method::Post,
        body: Some(b"hello".to_vec()),
        ..Request::get(server.origin())
    };
    fetch(&service, post, &scope).unwrap();
    assert_eq!(FORKS.load(Ordering::SeqCst), before);
}
