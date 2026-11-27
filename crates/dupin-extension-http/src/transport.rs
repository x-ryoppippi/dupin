use std::ffi::{CString, c_char};
use std::fs::File;
use std::io::{self, Read, Write};
use std::mem::MaybeUninit;
use std::net::IpAddr;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::ptr;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::url::{Host, Url};
use crate::{CancelToken, HttpError, Limits, Method};

const POLL: Duration = Duration::from_millis(20);
const STDERR_CAP: usize = 4096;

/// curl reads its options from this descriptor, so guest headers and the URL never appear in
/// its argument list, which any process of the same user can read.
const CONFIG_FD: RawFd = 3;
/// curl writes every response head here, keeping stdout for the body alone: on a shared stream
/// chunked trailers would land after the body and be read as part of it.
const HEADERS_FD: RawFd = 4;

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

/// A running curl. Dropping it kills curl, so a cancelled or capped fetch never leaves a
/// transfer running.
struct Curl {
    pid: libc::pid_t,
    status: Option<ExitStatus>,
}

impl Curl {
    /// Starts `program` with an empty environment, giving it each `(source, target)` descriptor
    /// as `target`.
    ///
    /// This is posix_spawn rather than `Command` with `pre_exec`, which forces fork(): on macOS
    /// the fork child of a multithreaded process runs libSystem's atfork handlers, and one that
    /// meets a lazy-initialization gate another thread held at fork time kills the child with
    /// SIGKILL before it reaches exec.
    fn spawn(
        program: &Path,
        arguments: &[String],
        descriptors: &[(BorrowedFd<'_>, RawFd)],
    ) -> io::Result<Self> {
        // A source may already sit on another pair's target number, so every source moves above
        // all targets before any target is filled.
        let above_targets = descriptors.iter().map(|&(_, target)| target).max().unwrap_or(2) + 1;
        let lifted = descriptors
            .iter()
            .map(|(source, _)| {
                // SAFETY: fcntl only duplicates a descriptor that `source` keeps open.
                let fd = unsafe { libc::fcntl(source.as_raw_fd(), libc::F_DUPFD_CLOEXEC, above_targets) };
                if fd < 0 {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: fcntl just returned this descriptor, and nothing else owns it.
                Ok(unsafe { OwnedFd::from_raw_fd(fd) })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let mut actions = FileActions::new()?;
        for (fd, &(_, target)) in lifted.iter().zip(descriptors) {
            actions.dup2(fd.as_raw_fd(), target)?;
        }
        let attributes = Attributes::new()?;

        let program = CString::new(program.as_os_str().as_bytes())?;
        let arguments = arguments
            .iter()
            .map(|argument| CString::new(argument.as_str()))
            .collect::<Result<Vec<_>, _>>()?;
        let argv: Vec<*mut c_char> = std::iter::once(&program)
            .chain(&arguments)
            .map(|argument| argument.as_ptr().cast_mut())
            .chain(std::iter::once(ptr::null_mut()))
            .collect();
        let envp: [*mut c_char; 1] = [ptr::null_mut()];
        let mut pid = 0;
        // SAFETY: every pointer is valid for the call, and argv and envp are null-terminated.
        let code = unsafe {
            libc::posix_spawn(
                &raw mut pid,
                program.as_ptr(),
                &raw const actions.0,
                &raw const attributes.0,
                argv.as_ptr(),
                envp.as_ptr(),
            )
        };
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code));
        }
        Ok(Self { pid, status: None })
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        let mut raw = 0;
        // SAFETY: waitpid writes only to `raw`.
        while unsafe { libc::waitpid(self.pid, &raw mut raw, 0) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
        let status = ExitStatus::from_raw(raw);
        self.status = Some(status);
        Ok(status)
    }
}

impl Drop for Curl {
    fn drop(&mut self) {
        if self.status.is_none() {
            // SAFETY: curl is not yet reaped, so its pid can't belong to another process.
            unsafe { libc::kill(self.pid, libc::SIGKILL) };
            let _ = self.wait();
        }
    }
}

struct FileActions(libc::posix_spawn_file_actions_t);

impl FileActions {
    fn new() -> io::Result<Self> {
        let mut actions = MaybeUninit::uninit();
        // SAFETY: init fills `actions`, which is used only once it succeeded.
        check(unsafe { libc::posix_spawn_file_actions_init(actions.as_mut_ptr()) })?;
        Ok(Self(unsafe { actions.assume_init() }))
    }

    fn dup2(&mut self, source: RawFd, target: RawFd) -> io::Result<()> {
        // SAFETY: `self.0` was initialized by `new`.
        check(unsafe { libc::posix_spawn_file_actions_adddup2(&raw mut self.0, source, target) })
    }
}

impl Drop for FileActions {
    fn drop(&mut self) {
        // SAFETY: `self.0` was initialized by `new` and is destroyed once.
        unsafe { libc::posix_spawn_file_actions_destroy(&raw mut self.0) };
    }
}

struct Attributes(libc::posix_spawnattr_t);

impl Attributes {
    /// Gives curl the default SIGPIPE disposition and an empty signal mask instead of the host's,
    /// and, on macOS, no descriptor beyond those the file actions name.
    fn new() -> io::Result<Self> {
        let mut attributes = MaybeUninit::uninit();
        // SAFETY: init fills `attributes`, which is used only once it succeeded.
        check(unsafe { libc::posix_spawnattr_init(attributes.as_mut_ptr()) })?;
        let mut this = Self(unsafe { attributes.assume_init() });
        #[cfg(target_vendor = "apple")]
        let only_named_descriptors = libc::POSIX_SPAWN_CLOEXEC_DEFAULT;
        #[cfg(not(target_vendor = "apple"))]
        let only_named_descriptors = 0;
        let flags = libc::POSIX_SPAWN_SETSIGDEF | libc::POSIX_SPAWN_SETSIGMASK | only_named_descriptors;
        let mut empty = MaybeUninit::uninit();
        let mut pipe = MaybeUninit::uninit();
        // SAFETY: each set is initialized by sigemptyset before it is read, and `this.0` by init.
        unsafe {
            libc::sigemptyset(empty.as_mut_ptr());
            libc::sigemptyset(pipe.as_mut_ptr());
            libc::sigaddset(pipe.as_mut_ptr(), libc::SIGPIPE);
            check(libc::posix_spawnattr_setsigmask(&raw mut this.0, empty.as_ptr()))?;
            check(libc::posix_spawnattr_setsigdefault(
                &raw mut this.0,
                pipe.as_ptr(),
            ))?;
            check(libc::posix_spawnattr_setflags(
                &raw mut this.0,
                flags as libc::c_short,
            ))?;
        }
        Ok(this)
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: `self.0` was initialized by `new` and is destroyed once.
        unsafe { libc::posix_spawnattr_destroy(&raw mut self.0) };
    }
}

fn check(code: libc::c_int) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code))
    }
}

/// A curl config file: one option per line, values quoted.
#[derive(Default)]
struct Config(String);

impl Config {
    fn flag(&mut self, name: &str) {
        self.0.push_str(name);
        self.0.push('\n');
    }

    fn option(&mut self, name: &str, value: &str) {
        self.0.push_str(name);
        self.0.push_str(" = \"");
        for c in value.chars() {
            match c {
                '\\' => self.0.push_str("\\\\"),
                '"' => self.0.push_str("\\\""),
                c => self.0.push(c),
            }
        }
        self.0.push_str("\"\n");
    }
}

impl Exchange<'_> {
    fn arguments() -> [String; 3] {
        // `--disable` must come first: it stops curl reading `.curlrc`.
        [
            "--disable".to_owned(),
            "--config".to_owned(),
            format!("/dev/fd/{CONFIG_FD}"),
        ]
    }

    fn config(&self, remaining: Duration) -> String {
        let limits = self.limits;
        let mut config = Config::default();
        for flag in ["silent", "show-error", "no-buffer", "globoff"] {
            config.flag(flag);
        }
        config.option("noproxy", "*");
        config.option("max-redirs", "0");
        config.option("proto", self.protocols());
        config.option("connect-timeout", &seconds(limits.connect_time.min(remaining)));
        config.option("max-time", &seconds(remaining));
        config.option("user-agent", self.user_agent);
        config.option("dump-header", &format!("/dev/fd/{HEADERS_FD}"));
        match self.method {
            // curl compares an advertised length with `max-filesize` even when no body follows.
            Method::Head => {
                config.flag("head");
                config.option("output", "/dev/null");
            }
            method => {
                config.option("max-filesize", &limits.max_response_bytes.to_string());
                if method != Method::Get {
                    config.option("request", method.as_str());
                }
            }
        }
        config.option("header", "Expect:");
        for (name, value) in self.headers {
            // `Name:` with nothing after it tells curl to drop the header; `Name;` sends it empty.
            config.option(
                "header",
                &if value.is_empty() {
                    format!("{name};")
                } else {
                    format!("{name}: {value}")
                },
            );
        }
        if self.body.is_some() {
            if !self
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            {
                config.option("header", "Content-Type:");
            }
            config.option("data-binary", "@-");
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
            config.option(
                "resolve",
                &format!("{name}:{}:{}", self.url.origin.port, pinned.join(",")),
            );
        }
        config.option("url", &self.url.to_string());
        config.0
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
        let start_failed =
            |e: io::Error| HttpError::Network(format!("couldn't start {}: {e}", self.curl.display()));
        let (stdin, body_writer) = match self.body {
            Some(_) => {
                let (reader, writer) = io::pipe().map_err(start_failed)?;
                (OwnedFd::from(reader), Some(writer))
            }
            None => (
                OwnedFd::from(File::open("/dev/null").map_err(start_failed)?),
                None,
            ),
        };
        let (stdout, stdout_writer) = io::pipe().map_err(start_failed)?;
        let (stderr, stderr_writer) = io::pipe().map_err(start_failed)?;
        let (config_reader, mut config_writer) = io::pipe().map_err(start_failed)?;
        let (headers_reader, headers_writer) = io::pipe().map_err(start_failed)?;
        let mut child = Curl::spawn(
            self.curl,
            &Self::arguments(),
            &[
                (stdin.as_fd(), libc::STDIN_FILENO),
                (stdout_writer.as_fd(), libc::STDOUT_FILENO),
                (stderr_writer.as_fd(), libc::STDERR_FILENO),
                (config_reader.as_fd(), CONFIG_FD),
                (headers_writer.as_fd(), HEADERS_FD),
            ],
        )
        .map_err(start_failed)?;
        // The child holds its own copies; ours would keep its output streams from ending.
        drop((stdin, stdout_writer, stderr_writer, config_reader, headers_writer));

        let config = self.config(remaining);
        thread::spawn(move || {
            let _ = config_writer.write_all(config.as_bytes());
        });
        if let (Some(mut body_writer), Some(body)) = (body_writer, self.body) {
            let body = body.to_vec();
            thread::spawn(move || {
                let _ = body_writer.write_all(&body);
            });
        }

        let (chunks, received) = mpsc::sync_channel::<Chunk>(4);
        let head_chunks = chunks.clone();
        thread::spawn(move || pump(headers_reader, Chunk::Head, head_chunks));
        thread::spawn(move || pump(stdout, Chunk::Body, chunks));
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
                Ok(chunk) => parser.feed(chunk)?,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let status = child.wait().map_err(|e| HttpError::Network(e.to_string()))?;
        let stderr = errors.join().unwrap_or_default();
        match status.code() {
            Some(0) => parser.finish(),
            Some(code) => Err(curl_error(code, stderr, self.limits)),
            None => Err(HttpError::Network(format!("curl was stopped by {status}"))),
        }
    }
}

enum Chunk {
    Head(Vec<u8>),
    Body(Vec<u8>),
}

fn pump(mut from: impl Read, tag: fn(Vec<u8>) -> Chunk, chunks: mpsc::SyncSender<Chunk>) {
    let mut buffer = vec![0; 16 * 1024];
    loop {
        match from.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if chunks.send(tag(buffer[..n].to_vec())).is_err() {
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

struct Parser<'a> {
    limits: &'a Limits,
    head: Vec<u8>,
    body: Vec<u8>,
}

impl<'a> Parser<'a> {
    fn new(limits: &'a Limits) -> Self {
        Self {
            limits,
            head: Vec::new(),
            body: Vec::new(),
        }
    }

    fn feed(&mut self, chunk: Chunk) -> Result<(), HttpError> {
        match chunk {
            Chunk::Head(bytes) => {
                if self.head.len() + bytes.len() > self.limits.max_header_bytes {
                    return Err(HttpError::HeadersTooLarge(self.limits.max_header_bytes));
                }
                self.head.extend_from_slice(&bytes);
            }
            Chunk::Body(bytes) => {
                if self.body.len() + bytes.len() > self.limits.max_response_bytes {
                    return Err(HttpError::ResponseTooLarge(self.limits.max_response_bytes));
                }
                self.body.extend_from_slice(&bytes);
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<Raw, HttpError> {
        let (status, headers) = final_head(&self.head)?;
        Ok(Raw {
            status,
            headers,
            body: self.body,
        })
    }
}

/// The final response head from curl's header stream, which holds interim 1xx heads before it
/// and chunked trailers after it.
fn final_head(mut stream: &[u8]) -> Result<(u16, Vec<(String, String)>), HttpError> {
    while let Some(end) = find_blank_line(stream) {
        let (status, headers) = parse_head(&stream[..end])?;
        if !(100..200).contains(&status) {
            return Ok((status, headers));
        }
        stream = &stream[end..];
    }
    Err(HttpError::InvalidResponse(
        "the response ended inside its headers".to_owned(),
    ))
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
