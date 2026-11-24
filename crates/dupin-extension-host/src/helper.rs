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

use dupin_js_runtime::{Acted, CataloguePage, Control, Diagnostic, Exports, Package, Row, RunError, Sources};

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
    let _reader = {
        let cancelled = cancelled.clone();
        std::thread::Builder::new()
            .name("extension-host-reader".into())
            .spawn(move || read(socket, &cancelled, &messages))?
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
        let answer = match execute(entry, modules, call, control) {
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
            Err(RunError::Cancelled) => Answer::Cancelled,
            Err(RunError::Failed(diagnostic)) => Answer::Failed { diagnostic },
        };
        wire::write(&mut writer, &FromHelper::Done { id, answer }, &[])?;
    }
    Ok(())
}

/// Forwards every message but `cancel`, which it applies at once so a running call sees it.
/// When Dupin goes away, so does the helper, even with a call still running.
fn read(mut socket: UnixStream, cancelled: &AtomicU64, messages: &mpsc::SyncSender<(ToHelper, Vec<u8>)>) {
    loop {
        match wire::read::<ToHelper>(&mut socket) {
            Ok(Some((ToHelper::Cancel { id }, _))) => cancelled.store(id, Ordering::Relaxed),
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
}

fn execute(
    entry: String,
    modules: Vec<(String, &[u8])>,
    call: Call,
    control: Control,
) -> Result<Outcome, RunError> {
    match call {
        Call::Compile => {
            let files = modules
                .into_iter()
                .map(|(name, source)| (name, source.to_vec()))
                .collect();
            let sources = Sources::new(entry, files)?;
            Package::compile(sources, control).map(Outcome::Compiled)
        }
        Call::Check => load(entry, modules).check(control).map(Outcome::Checked),
        Call::Search { query, limit } => load(entry, modules)
            .search(&query, limit as usize, control)
            .map(Outcome::Rows),
        Call::Items { cursor, reason } => load(entry, modules)
            .items(cursor.as_deref(), reason, control)
            .map(Outcome::Catalogue),
        Call::Action { handler, data } => load(entry, modules)
            .action(&handler, data.as_ref(), control)
            .map(Outcome::Acted),
    }
}

/// The package a call carries as bytecode.
fn load(entry: String, modules: Vec<(String, &[u8])>) -> Package {
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
    unsafe { Package::from_compiled(entry, bytecode) }
}
