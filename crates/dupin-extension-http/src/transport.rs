use std::io::{Read, Write};
use std::net::IpAddr;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::url::{Host, Url};
use crate::{CancelToken, HttpError, Limits, Method};

const POLL: Duration = Duration::from_millis(20);
const STDERR_CAP: usize = 4096;

pub(crate) struct Exchange<'a> {
    pub(crate) curl: &'a std::path::Path,
    pub(crate) user_agent: &'a str,
    pub(crate) limits: &'a Limits,
    pub(crate) deadline: Instant,
    pub(crate) cancel: &'a CancelToken,
    pub(crate) url: &'a Url,
    pub(crate) addresses: &'a [IpAddr],
    pub(crate) method: Method,
    pub(crate) headers: &'a [(String, String)],
    pub(crate) body: Option<&'a [u8]>,
}

pub(crate) struct Raw {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

/// Kills curl when an exchange ends early, so a cancelled or capped fetch never leaves a
/// transfer running.
struct Reaper(Child);

impl Drop for Reaper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Exchange<'_> {
    fn arguments(&self, remaining: Duration) -> Vec<String> {
        let limits = self.limits;
        // `--disable` must come first: it stops curl reading `.curlrc`.
        let mut args: Vec<String> = [
            "--disable",
            "--silent",
            "--show-error",
            "--no-buffer",
            "--globoff",
            "--noproxy",
            "*",
            "--max-redirs",
            "0",
        ]
        .map(str::to_owned)
        .to_vec();
        args.push("--proto".into());
        args.push(self.protocols().into());
        args.push("--connect-timeout".into());
        args.push(seconds(limits.connect_time.min(remaining)));
        args.push("--max-time".into());
        args.push(seconds(remaining));
        args.push("--max-filesize".into());
        args.push(limits.max_response_bytes.to_string());
        args.push("--user-agent".into());
        args.push(self.user_agent.into());
        match self.method {
            Method::Head => args.push("--head".into()),
            Method::Get => args.push("--include".into()),
            method => {
                args.push("--include".into());
                args.push("--request".into());
                args.push(method.as_str().into());
            }
        }
        args.push("--header".into());
        args.push("Expect:".into());
        for (name, value) in self.headers {
            args.push("--header".into());
            // `Name:` with nothing after it tells curl to drop the header; `Name;` sends it empty.
            args.push(if value.is_empty() {
                format!("{name};")
            } else {
                format!("{name}: {value}")
            });
        }
        if self.body.is_some() {
            if !self
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            {
                args.push("--header".into());
                args.push("Content-Type:".into());
            }
            args.push("--data-binary".into());
            args.push("@-".into());
        }
        if let Host::Domain(name) = &self.url.origin.host {
            let pinned: Vec<String> = self
                .addresses
                .iter()
                .map(|ip| match ip {
                    IpAddr::V4(ip) => ip.to_string(),
                    IpAddr::V6(ip) => format!("[{ip}]"),
                })
                .collect();
            args.push("--resolve".into());
            args.push(format!("{name}:{}:{}", self.url.origin.port, pinned.join(",")));
        }
        args.push("--url".into());
        args.push(self.url.to_string());
        args
    }

    fn protocols(&self) -> &'static str {
        #[cfg(test)]
        if self.url.origin.scheme == crate::url::Scheme::Http {
            return "=http";
        }
        "=https"
    }

    pub(crate) fn run(self) -> Result<Raw, HttpError> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(HttpError::Timeout);
        }
        let mut child = Command::new(self.curl)
            .args(self.arguments(remaining))
            .env_clear()
            .stdin(if self.body.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| HttpError::Network(format!("couldn't start {}: {e}", self.curl.display())))?;
        if let (Some(mut stdin), Some(body)) = (child.stdin.take(), self.body) {
            let body = body.to_vec();
            thread::spawn(move || {
                let _ = stdin.write_all(&body);
            });
        }
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let mut child = Reaper(child);

        let (chunks, received) = mpsc::sync_channel::<Vec<u8>>(4);
        thread::spawn(move || pump(stdout, chunks));
        let errors = thread::spawn(move || {
            let mut text = Vec::new();
            let _ = stderr.take(STDERR_CAP as u64).read_to_end(&mut text);
            String::from_utf8_lossy(&text).trim().to_owned()
        });

        let mut parser = Parser::new(self.limits);
        loop {
            if self.cancel.is_cancelled() {
                return Err(HttpError::Cancelled);
            }
            let now = Instant::now();
            if now >= self.deadline {
                return Err(HttpError::Timeout);
            }
            match received.recv_timeout(POLL.min(self.deadline - now)) {
                Ok(chunk) => parser.feed(&chunk)?,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let status = child.0.wait().map_err(|e| HttpError::Network(e.to_string()))?;
        let stderr = errors.join().unwrap_or_default();
        match status.code() {
            Some(0) => parser.finish(),
            Some(code) => Err(curl_error(code, stderr, self.limits)),
            None => Err(HttpError::Network("curl was stopped by a signal".to_owned())),
        }
    }
}

fn pump(mut stdout: impl Read, chunks: mpsc::SyncSender<Vec<u8>>) {
    let mut buffer = vec![0; 16 * 1024];
    loop {
        match stdout.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if chunks.send(buffer[..n].to_vec()).is_err() {
                    return;
                }
            }
        }
    }
}

fn seconds(duration: Duration) -> String {
    format!("{:.3}", duration.as_secs_f64().max(0.001))
}

fn curl_error(code: i32, stderr: String, limits: &Limits) -> HttpError {
    match code {
        6 => HttpError::Dns(stderr),
        7 => HttpError::Connect(stderr),
        28 => HttpError::Timeout,
        63 => HttpError::ResponseTooLarge(limits.max_response_bytes),
        35 | 51 | 53 | 54 | 58 | 59 | 60 | 64 | 66 | 77 | 80 | 82 | 83 | 90 | 91 => HttpError::Tls(stderr),
        _ => HttpError::Network(if stderr.is_empty() {
            format!("curl exited with {code}")
        } else {
            stderr
        }),
    }
}

enum Stage {
    Headers,
    Body {
        status: u16,
        headers: Vec<(String, String)>,
    },
}

struct Parser<'a> {
    limits: &'a Limits,
    stage: Stage,
    head: Vec<u8>,
    header_bytes: usize,
    body: Vec<u8>,
}

impl<'a> Parser<'a> {
    fn new(limits: &'a Limits) -> Self {
        Self {
            limits,
            stage: Stage::Headers,
            head: Vec::new(),
            header_bytes: 0,
            body: Vec::new(),
        }
    }

    fn feed(&mut self, mut chunk: &[u8]) -> Result<(), HttpError> {
        while !chunk.is_empty() {
            match &self.stage {
                Stage::Headers => {
                    let before = self.head.len();
                    self.head.extend_from_slice(chunk);
                    let Some(end) = find_blank_line(&self.head) else {
                        self.header_bytes += chunk.len();
                        if self.header_bytes > self.limits.max_header_bytes {
                            return Err(HttpError::HeadersTooLarge(self.limits.max_header_bytes));
                        }
                        return Ok(());
                    };
                    let consumed = end - before;
                    self.header_bytes += consumed;
                    if self.header_bytes > self.limits.max_header_bytes {
                        return Err(HttpError::HeadersTooLarge(self.limits.max_header_bytes));
                    }
                    let block = std::mem::take(&mut self.head);
                    let (status, headers) = parse_head(&block[..end])?;
                    chunk = &chunk[consumed..];
                    // An interim 1xx response precedes the real one on the same stream.
                    if !(100..200).contains(&status) {
                        self.stage = Stage::Body { status, headers };
                    }
                }
                Stage::Body { .. } => {
                    if self.body.len() + chunk.len() > self.limits.max_response_bytes {
                        return Err(HttpError::ResponseTooLarge(self.limits.max_response_bytes));
                    }
                    self.body.extend_from_slice(chunk);
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<Raw, HttpError> {
        match self.stage {
            Stage::Body { status, headers } => Ok(Raw {
                status,
                headers,
                body: self.body,
            }),
            Stage::Headers => Err(HttpError::InvalidResponse(
                "the response ended inside its headers".to_owned(),
            )),
        }
    }
}

fn find_blank_line(bytes: &[u8]) -> Option<usize> {
    let crlf = bytes.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4);
    let lf = bytes.windows(2).position(|w| w == b"\n\n").map(|i| i + 2);
    match (crlf, lf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn parse_head(block: &[u8]) -> Result<(u16, Vec<(String, String)>), HttpError> {
    let text = String::from_utf8_lossy(block);
    let mut lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line));
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .strip_prefix("HTTP/")
        .and_then(|rest| rest.split(' ').nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .filter(|code| (100..1000).contains(code))
        .ok_or_else(|| HttpError::InvalidResponse(format!("bad status line `{status_line}`")))?;
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = headers.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    Ok((status, headers))
}
