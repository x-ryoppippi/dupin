//! The helper process: Dupin's own executable, started with [`crate::FLAG`]. It reads
//! messages on standard input, a Unix socket whose other end Dupin holds, and compiles and runs
//! packages there with `dupin-js-runtime`.

use std::io;
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dupin_js_runtime::{
    Acted, Bridge, CataloguePage, Control, Diagnostic, Environment, Exports, Package, Reply, Row, RunError,
    Sources,
};

use crate::sandbox;
use crate::wire::{self, Answer, Call, FromHelper, ToHelper};

unsafe extern "C" {
    fn tzset();
}

/// Serves Dupin until it closes the socket, then exits.
pub fn serve() -> ! {
    // SAFETY: Dupin starts the helper with the socket as standard input, and nothing else in
    // this process owns descriptor 0.
    let socket = unsafe { UnixStream::from_raw_fd(0) };
    let code = match run(socket) {
        Ok(()) => 0,
        Err(_) => 1,
    };
    std::process::exit(code)
}

fn run(socket: UnixStream) -> io::Result<()> {
    let mut writer = socket.try_clone()?;
    let cancelled = Arc::new(AtomicU64::new(0));
    let (messages, received) = mpsc::sync_channel(4);
    let (replies, answered) = mpsc::sync_channel(dupin_js_runtime::MAX_PENDING);
    let _reader = {
        let cancelled = cancelled.clone();
        std::thread::Builder::new()
            .name("extension-host-reader".into())
            .spawn(move || read(socket, &cancelled, &messages, &replies))?
    };
    // The C library reads the time zone on first use, which the sandbox would deny, leaving
    // every local-time `Date` method in UTC.
    // SAFETY: no other thread of this process reads or sets the time zone.
    unsafe { tzset() };
    if let Err(reason) = sandbox::enter() {
        wire::write(&mut writer, &FromHelper::Refused { reason }, &[])?;
        return Err(io::Error::other("no sandbox"));
    }
    sandbox::limit_resources();
    wire::write(
        &mut writer,
        &FromHelper::Hello {
            version: wire::VERSION,
            engine: dupin_js_runtime::engine_version().into(),
        },
        &[],
    )?;
    for (message, tail) in received {
        let ToHelper::Run {
            id,
            deadline_ms,
            entry,
            modules,
            call,
            environment,
        } = message
        else {
            continue;
        };
        let modules = wire::split(modules, &tail)?;
        let cancelled = cancelled.clone();
        let control = Control {
            deadline: Instant::now() + Duration::from_millis(deadline_ms),
            is_cancelled: Box::new(move || cancelled.load(Ordering::Relaxed) == id),
        };
        let mut bridge = HelperBridge {
            id,
            writer: &mut writer,
            replies: &answered,
        };
        let answer = match execute(entry, modules, call, environment, control, &mut bridge) {
            Ok(Outcome::Compiled(package)) => {
                let (modules, tail) = wire::spans(package.modules());
                let bytes: usize = tail.iter().map(|part| part.len()).sum();
                if bytes <= wire::MAX_TAIL_BYTES {
                    let answer = Answer::Compiled { modules };
                    wire::write(&mut writer, &FromHelper::Done { id, answer }, &tail)?;
                    continue;
                }
                Answer::Failed {
                    diagnostic: Diagnostic::new(
                        "package-too-large",
                        "the compiled package is larger than the extension host can return",
                    )
                    .file(package.entry())
                    .hint("Bundle and minify the extension, and leave out code it doesn't run."),
                }
            }
            Ok(Outcome::Checked(exports)) => Answer::Checked { exports },
            Ok(Outcome::Rows(items)) => Answer::Rows { items },
            Ok(Outcome::Catalogue(page)) => Answer::Catalogue { page },
            Ok(Outcome::Acted(acted)) => Answer::Acted { acted },
            Ok(Outcome::Handled) => Answer::Handled,
            Err(RunError::Cancelled) => Answer::Cancelled,
            Err(RunError::Failed(diagnostic)) => Answer::Failed { diagnostic },
        };
        wire::write(&mut writer, &FromHelper::Done { id, answer }, &[])?;
        dupin_js_runtime::prepare();
    }
    Ok(())
}

/// Carries one run's host requests to Dupin and its replies back.
struct HelperBridge<'a> {
    id: u64,
    writer: &'a mut UnixStream,
    replies: &'a mpsc::Receiver<(u64, u32, Vec<u8>)>,
}

impl Bridge for HelperBridge<'_> {
    fn send(&mut self, call: u32, op: &str, args: &str) -> Result<(), Diagnostic> {
        let message = FromHelper::Host {
            id: self.id,
            call,
            op: op.into(),
        };
        wire::write(self.writer, &message, &[args.as_bytes()])
            .map_err(|error| Diagnostic::new("host-error", format!("couldn't reach Dupin: {error}")))
    }

    fn receive(&mut self, stop: &dyn Fn() -> bool) -> Option<Reply> {
        loop {
            match self.replies.recv_timeout(TICK) {
                Ok((id, call, json)) if id == self.id => {
                    let json = String::from_utf8(json).unwrap_or_else(|_| {
                        r#"{"error":{"code":"host-error","message":"Dupin's reply wasn't UTF-8"}}"#.into()
                    });
                    return Some(Reply { call, json });
                }
                // A reply to a run that has ended, which nothing waits for any more.
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            }
            if stop() {
                return None;
            }
        }
    }
}

const TICK: Duration = Duration::from_millis(5);

/// Forwards runs to the main loop and replies to the running call's bridge, and applies `cancel`
/// at once so a running call sees it. When Dupin goes away, so does the helper, even with a
/// call still running.
fn read(
    mut socket: UnixStream,
    cancelled: &AtomicU64,
    messages: &mpsc::SyncSender<(ToHelper, Vec<u8>)>,
    replies: &mpsc::SyncSender<(u64, u32, Vec<u8>)>,
) {
    loop {
        match wire::read::<ToHelper>(&mut socket) {
            Ok(Some((ToHelper::Cancel { id }, _))) => cancelled.store(id, Ordering::Relaxed),
            Ok(Some((ToHelper::Reply { id, call }, json))) => {
                if replies.send((id, call, json)).is_err() {
                    return;
                }
            }
            Ok(Some(message)) => {
                if messages.send(message).is_err() {
                    return;
                }
            }
            Ok(None) | Err(_) => std::process::exit(0),
        }
    }
}

enum Outcome {
    Compiled(Package),
    Checked(Exports),
    Rows(Vec<Row>),
    Catalogue(CataloguePage),
    Acted(Acted),
    Handled,
}

fn execute(
    entry: String,
    modules: Vec<(String, &[u8])>,
    call: Call,
    environment: Option<Environment>,
    control: Control,
    bridge: &mut dyn Bridge,
) -> Result<Outcome, RunError> {
    if let Call::Compile = call {
        let files = modules
            .into_iter()
            .map(|(name, source)| (name, source.to_vec()))
            .collect();
        let sources = Sources::new(entry, files)?;
        return Package::compile(sources, control).map(Outcome::Compiled);
    }
    let bytecode = modules
        .into_iter()
        .map(|(name, bytes)| (name, Box::from(bytes)))
        .collect();
    // SAFETY: only Dupin writes to this socket. The only bytecode it sends is what a `compiled`
    // answer gave it from a helper of its own build, after that helper stated this engine
    // version, and it never makes or changes bytecode itself. That helper was started to
    // compile this package alone, so these are bytes `Package::compile` wrote from this
    // package's source, or ones that source subverted its own compile into writing, which can
    // do no more than running the source could.
    let package = unsafe { Package::from_compiled(entry, bytecode) }.with_environment(environment);
    match call {
        Call::Compile => unreachable!("compiled above"),
        Call::Check => package.check(control).map(Outcome::Checked),
        Call::Search { query, limit } => package
            .search_with(&query, limit as usize, control, bridge)
            .map(Outcome::Rows),
        Call::Items { cursor, reason } => package
            .items(cursor.as_deref(), reason, control)
            .map(Outcome::Catalogue),
        Call::Action { name, data } => package
            .action(&name, &data.to_string(), control, bridge)
            .map(Outcome::Acted),
        Call::Event { name, event } => package
            .event(&name, &event.to_string(), control, bridge)
            .map(|()| Outcome::Handled),
    }
}
