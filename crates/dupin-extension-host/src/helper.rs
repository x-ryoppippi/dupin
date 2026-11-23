//! The helper process: Dupin's own executable, started with [`crate::FLAG`]. It reads
//! messages on standard input, a Unix socket whose other end Dupin holds, and runs packages
//! there with `dupin-js-runtime`.

use std::collections::HashMap;
use std::io;
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dupin_js_runtime::{Control, Diagnostic, Package, RunError};

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
    let mut packages = HashMap::new();
    for (message, tail) in received {
        match message {
            ToHelper::Load {
                package,
                entry,
                modules,
            } => {
                packages.insert(package, rebuild(entry, modules, tail)?);
            }
            ToHelper::Unload { package } => {
                packages.remove(&package);
            }
            ToHelper::Run {
                id,
                package,
                deadline_ms,
                call,
            } => {
                let answer = match packages.get(&package) {
                    Some(package) => execute(package, call, id, deadline_ms, &cancelled),
                    None => Answer::Failed {
                        diagnostic: Diagnostic::new(
                            "host-error",
                            "the package wasn't sent to the extension host",
                        ),
                    },
                };
                wire::write(&mut writer, &FromHelper::Done { id, answer }, &[])?;
            }
            ToHelper::Cancel { .. } => {}
        }
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

fn rebuild(entry: String, modules: Vec<wire::ModuleSpan>, tail: Vec<u8>) -> io::Result<Package> {
    let mut bytecode = HashMap::with_capacity(modules.len());
    let mut rest = &tail[..];
    for module in modules {
        let len = module.len as usize;
        if len > rest.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "short package"));
        }
        let (bytes, after) = rest.split_at(len);
        bytecode.insert(module.name, Box::from(bytes));
        rest = after;
    }
    // SAFETY: only Dupin writes to this socket, and it sends bytecode that `Package::load`
    // compiled, after this helper stated the same engine version in its hello.
    Ok(unsafe { Package::from_compiled(entry, bytecode) })
}

fn execute(package: &Package, call: Call, id: u64, deadline_ms: u64, cancelled: &Arc<AtomicU64>) -> Answer {
    let cancelled = cancelled.clone();
    let control = Control {
        deadline: Instant::now() + Duration::from_millis(deadline_ms),
        is_cancelled: Box::new(move || cancelled.load(Ordering::Relaxed) == id),
    };
    let result = match call {
        Call::Check => package.check(control).map(|()| Answer::Checked),
        Call::Search { query, limit } => package
            .search(&query, limit as usize, control)
            .map(|items| Answer::Rows { items }),
    };
    match result {
        Ok(answer) => answer,
        Err(RunError::Cancelled) => Answer::Cancelled,
        Err(RunError::Failed(diagnostic)) => Answer::Failed { diagnostic },
    }
}
