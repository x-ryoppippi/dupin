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
    Acted, Bridge, CataloguePage, Control, Diagnostic, Engine, Exports, Handled, Package, Reply, Row,
    RunError, Sources,
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
    let mut held = Held::default();
    let mut next = None;
    let mut ran = false;
    loop {
        // Made while no call waits, so that making it isn't part of the next call's latency. A
        // helper that never ran a package, such as one that compiles, never needs one.
        if ran && next.is_none() {
            next = Engine::new().ok();
        }
        let Ok((message, tail)) = received.recv() else {
            return Ok(());
        };
        let ToHelper::Run {
            id,
            deadline_ms,
            entry,
            package,
            modules,
            call,
            environment,
        } = message
        else {
            continue;
        };
        let modules = modules.map(|modules| wire::split(modules, &tail)).transpose()?;
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
        let mut spent = None;
        let outcome = match (call, modules) {
            (Call::Compile, Some(modules)) => compile(entry, modules, control),
            (call, modules) => match held.package(package, entry, modules) {
                Some(package) => next.take().map_or_else(Engine::new, Ok).and_then(|engine| {
                    ran = true;
                    let package = package.with_environment(environment);
                    execute(&package, call, control, &mut bridge, spent.insert(engine))
                }),
                None => {
                    let answer = Answer::Missing;
                    wire::write(&mut writer, &FromHelper::Done { id, answer }, &[])?;
                    continue;
                }
            },
        };
        let answer = match outcome {
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
            Ok(Outcome::Handled(handled)) => Answer::Handled { handled },
            Err(RunError::Cancelled) => Answer::Cancelled,
            Err(RunError::Failed(diagnostic)) => Answer::Failed { diagnostic },
        };
        wire::write(&mut writer, &FromHelper::Done { id, answer }, &[])?;
        // Freed once the answer is on its way, which is as much of a call's cost as making it.
        drop(spent);
    }
}

/// The bytecode of packages Dupin ran here, by the name Dupin gives each compile, so that a later
/// run names its package instead of sending it again. Only bytes Dupin sent go in, under names
/// only Dupin chooses, and they go with this process, so holding them lets no guest reach
/// anything it couldn't already reach here. Each run still loads them into an engine of its own.
#[derive(Default)]
struct Held(wire::Held<Package>);

impl Held {
    /// The package a run names: the one it carries, which is then held if it is named, or the
    /// one held under its name.
    fn package(
        &mut self,
        name: Option<u64>,
        entry: String,
        modules: Option<Vec<(String, &[u8])>>,
    ) -> Option<Package> {
        let Some(modules) = modules else {
            return Some(self.0.run(name?)?.with_environment(None));
        };
        let bytes = modules.iter().map(|(_, bytecode)| bytecode.len()).sum();
        let bytecode = modules
            .into_iter()
            .map(|(name, bytes)| (name, Box::from(bytes)))
            .collect();
        // SAFETY: only Dupin writes to this socket. The only bytecode it sends is what a
        // `compiled` answer gave it from a helper of its own build, after that helper stated this
        // engine version, and it never makes or changes bytecode itself. That helper was started
        // to compile this package alone, so these are bytes `Package::compile` wrote from this
        // package's source, or ones that source subverted its own compile into writing, which can
        // do no more than running the source could.
        let package = unsafe { Package::from_compiled(entry, bytecode) };
        if let Some(name) = name {
            self.0.hold(name, package.with_environment(None), bytes);
        }
        Some(package)
    }
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
    socket: UnixStream,
    cancelled: &AtomicU64,
    messages: &mpsc::SyncSender<(ToHelper, Vec<u8>)>,
    replies: &mpsc::SyncSender<(u64, u32, Vec<u8>)>,
) {
    let mut socket = io::BufReader::new(socket);
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
    Handled(Handled),
}

fn compile(entry: String, modules: Vec<(String, &[u8])>, control: Control) -> Result<Outcome, RunError> {
    let files = modules
        .into_iter()
        .map(|(name, source)| (name, source.to_vec()))
        .collect();
    let sources = Sources::new(entry, files)?;
    Package::compile(sources, control).map(Outcome::Compiled)
}

fn execute(
    package: &Package,
    call: Call,
    control: Control,
    bridge: &mut dyn Bridge,
    engine: &Engine,
) -> Result<Outcome, RunError> {
    match call {
        Call::Compile => Err(RunError::Failed(Diagnostic::new(
            "host-error",
            "a compile carries the package's source",
        ))),
        Call::Check => package.check(engine, control).map(Outcome::Checked),
        Call::Search { query, limit } => package
            .search_with(engine, &query, limit as usize, control, bridge)
            .map(Outcome::Rows),
        Call::Items { cursor, reason } => package
            .items(engine, cursor.as_deref(), reason, control, bridge)
            .map(Outcome::Catalogue),
        Call::Action { name, data } => package
            .action(engine, &name, &data.to_string(), control, bridge)
            .map(Outcome::Acted),
        Call::Event { name, event } => package
            .event(engine, &name, &event.to_string(), control, bridge)
            .map(Outcome::Handled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: usize = 1024 * 1024;

    fn carried(bytes: &'static [u8]) -> Option<Vec<(String, &'static [u8])>> {
        Some(vec![("main.js".into(), bytes)])
    }

    #[test]
    fn the_least_recently_run_package_goes_first_and_one_too_large_is_never_held() {
        let four = vec![0; 4 * MIB].leak();
        let three = vec![0; 3 * MIB].leak();
        let nine = vec![0; 9 * MIB].leak();
        let mut held = Held::default();
        let mut run = |name: u64, modules| held.package(Some(name), "main.js".into(), modules).is_some();
        assert!(run(1, carried(four)));
        assert!(run(2, carried(three)));
        assert!(run(1, None), "1 ran last");
        assert!(run(3, carried(three)));
        assert!(!run(2, None), "2 went to make room for 3");
        assert!(run(1, None) && run(3, None));
        assert!(run(4, carried(nine)), "a package too large to hold still runs");
        assert!(!run(4, None));
        assert!(run(1, None) && run(3, None));
    }

    #[test]
    fn a_run_that_names_no_package_is_never_held() {
        let mut held = Held::default();
        assert!(held.package(None, "main.js".into(), carried(b"x")).is_some());
        assert!(held.package(None, "main.js".into(), None).is_none());
    }
}
