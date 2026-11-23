use dupin_core::script::{LineDecoder, OutputBuffer, ScriptRun, Stream};
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Read},
    os::fd::{AsRawFd, RawFd},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{ChildStderr, Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub type RunId = u64;
pub struct Start {
    pub id: RunId,
    pub refused: bool,
}

type Notify = Arc<dyn Fn(RunId, bool) + Send + Sync>;
type Signal = Arc<dyn Fn(i32, i32) + Send + Sync>;
const EXIT_DRAIN: Duration = Duration::from_millis(100);
const CANCEL_GRACE: Duration = Duration::from_secs(2);
const POLL_INTERVAL_MS: i32 = 20;
const SHELL_LIMIT: usize = 4;
const SHELL_TAIL_BYTES: usize = 512;
const SHELL_TAIL_LINES: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Exited(Option<i32>),
    Cancelled,
    Failed(String),
}

impl Status {
    pub(crate) fn failed(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Exited(None))
            || matches!(self, Self::Exited(Some(code)) if *code != 0)
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => f.write_str("Running"),
            Self::Exited(Some(0)) => f.write_str("Finished"),
            Self::Exited(Some(code)) => write!(f, "Exited with code {code}"),
            Self::Exited(None) => f.write_str("Terminated by signal"),
            Self::Cancelled => f.write_str("Cancelled"),
            Self::Failed(problem) => write!(f, "Failed: {problem}"),
        }
    }
}

#[derive(Debug)]
pub struct Snapshot {
    pub id: RunId,
    pub command: Arc<ScriptRun>,
    pub status: Status,
    pub output: OutputBuffer,
    pub elapsed: Duration,
}

impl Snapshot {
    pub(crate) fn summary(&self) -> String {
        format!("{} · {:.1} s", self.status, self.elapsed.as_secs_f64())
    }
}

struct Run {
    id: RunId,
    command: Arc<ScriptRun>,
    started: Instant,
    cancelled: AtomicBool,
    snapshot: Mutex<Arc<Snapshot>>,
}

#[derive(Default)]
struct LifecycleState {
    shutting_down: bool,
    groups: HashMap<RunId, Group>,
}

struct Group {
    pgid: i32,
    terminated: bool,
    killed: bool,
}

struct Lifecycle {
    state: Mutex<LifecycleState>,
    signal: Signal,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Lifecycle {
    fn spawn<T>(
        &self,
        id: RunId,
        cancelled: &AtomicBool,
        spawn: impl FnOnce() -> io::Result<(T, i32)>,
    ) -> io::Result<Option<T>> {
        let mut state = lock(&self.state);
        if state.shutting_down || cancelled.load(Ordering::Acquire) {
            return Ok(None);
        }
        let (child, pgid) = spawn()?;
        state.groups.insert(
            id,
            Group {
                pgid,
                terminated: false,
                killed: false,
            },
        );
        Ok(Some(child))
    }

    fn cancel(&self, id: RunId, force: bool) {
        let mut state = lock(&self.state);
        if let Some(group) = state.groups.get_mut(&id) {
            if force && !group.killed {
                (self.signal)(group.pgid, libc::SIGKILL);
                group.killed = true;
            } else if !force && !group.terminated && !group.killed {
                (self.signal)(group.pgid, libc::SIGTERM);
                group.terminated = true;
            }
        }
    }

    fn kill_all_now(&self) {
        let mut state = lock(&self.state);
        state.shutting_down = true;
        for group in state.groups.values_mut() {
            if !group.killed {
                (self.signal)(group.pgid, libc::SIGKILL);
                group.killed = true;
            }
        }
    }

    fn reaped(&self, id: RunId) {
        lock(&self.state).groups.remove(&id);
    }

    fn reap<T>(
        &self,
        id: RunId,
        leader_exited: impl FnOnce(i32) -> io::Result<bool>,
        poll: impl FnOnce() -> io::Result<Option<T>>,
    ) -> io::Result<Option<(T, bool)>> {
        let mut state = lock(&self.state);
        let mut cancelled = false;
        if let Some(group) = state.groups.get_mut(&id) {
            if group.terminated && !group.killed {
                if !leader_exited(group.pgid)? {
                    return Ok(None);
                }
                // The unreaped leader reserves the group ID while we kill any survivors.
                (self.signal)(group.pgid, libc::SIGKILL);
                group.killed = true;
            }
            cancelled = group.terminated;
        }
        let result = poll()?;
        if result.is_some() {
            state.groups.remove(&id);
        }
        Ok(result.map(|status| (status, cancelled)))
    }
}

struct Inner {
    lifecycle: Lifecycle,
    runs: Mutex<VecDeque<Arc<Run>>>,
    next: AtomicU64,
    running_scripts: Arc<AtomicUsize>,
    running_shells: AtomicUsize,
    notify: Notify,
    home: PathBuf,
}

pub struct Runner {
    inner: Arc<Inner>,
}

impl Runner {
    pub fn new(home: PathBuf, notify: Notify) -> Self {
        Self {
            inner: Arc::new(Inner {
                lifecycle: Lifecycle {
                    state: Mutex::default(),
                    signal: Arc::new(signal_group),
                },
                runs: Mutex::default(),
                next: AtomicU64::new(1),
                running_scripts: Arc::default(),
                running_shells: AtomicUsize::default(),
                notify,
                home,
            }),
        }
    }

    pub fn activity(&self) -> &Arc<AtomicUsize> {
        &self.inner.running_scripts
    }

    fn insert_run(&self, runs: &mut VecDeque<Arc<Run>>, command: ScriptRun, status: Status) -> Arc<Run> {
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let command = Arc::new(command);
        let run = Arc::new(Run {
            id,
            command: command.clone(),
            started: Instant::now(),
            cancelled: AtomicBool::new(false),
            snapshot: Mutex::new(Arc::new(Snapshot {
                id,
                command,
                status,
                output: OutputBuffer::default(),
                elapsed: Duration::ZERO,
            })),
        });
        while runs.len() >= 20 {
            let index = runs
                .iter()
                .position(|run| lock(&run.snapshot).status != Status::Running)
                .unwrap();
            runs.remove(index);
        }
        runs.push_front(run.clone());
        run
    }

    fn refused(&self, runs: &mut VecDeque<Arc<Run>>, command: ScriptRun, problem: String) -> Start {
        Start {
            id: self.insert_run(runs, command, Status::Failed(problem)).id,
            refused: true,
        }
    }

    pub fn refuse(&self, command: ScriptRun, problem: String) -> Start {
        self.refused(&mut lock(&self.inner.runs), command, problem)
    }

    pub fn start(&self, command: ScriptRun) -> Start {
        let mut runs = lock(&self.inner.runs);
        if lock(&self.inner.lifecycle.state).shutting_down {
            return self.refused(&mut runs, command, "Dupin is shutting down.".into());
        }
        if runs
            .iter()
            .filter(|run| lock(&run.snapshot).status == Status::Running)
            .count()
            >= 8
        {
            return self.refused(&mut runs, command, "Eight scripts are already running.".into());
        }
        let run = self.insert_run(&mut runs, command, Status::Running);
        let id = run.id;
        drop(runs);
        self.inner.running_scripts.fetch_add(1, Ordering::AcqRel);
        let inner = self.inner.clone();
        let worker_run = run.clone();
        if let Err(error) = thread::Builder::new()
            .name(format!("dupin-script-{id}"))
            .spawn(move || {
                let mut command = Command::new(&worker_run.command.path);
                command.args(&worker_run.command.args).current_dir(&inner.home);
                let mut output = OutputBuffer::default();
                let status = execute(
                    &inner.lifecycle,
                    id,
                    &worker_run.cancelled,
                    command,
                    &mut Capture::Script(&mut output),
                    |output| publish(&inner, &worker_run, Status::Running, output.clone()),
                );
                publish(&inner, &worker_run, status, output);
            })
        {
            publish(
                &self.inner,
                &run,
                Status::Failed(error.to_string()),
                OutputBuffer::default(),
            );
        }
        Start { id, refused: false }
    }

    pub fn start_shell(
        &self,
        line: &str,
        complete: impl FnOnce(Result<(), String>) + Send + 'static,
    ) -> Result<(), String> {
        let home = std::env::home_dir().ok_or("Couldn't determine the home directory.")?;
        let shell = std::env::var_os("SHELL")
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "/bin/zsh".into());
        let mut command = Command::new(shell);
        command.args(["-lc", line]).current_dir(home);
        self.start_shell_command(command, complete)
    }

    fn start_shell_command(
        &self,
        command: Command,
        complete: impl FnOnce(Result<(), String>) + Send + 'static,
    ) -> Result<(), String> {
        let inner = self.inner.clone();
        inner
            .running_shells
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < SHELL_LIMIT).then_some(count + 1)
            })
            .map_err(|_| {
                "Four hotkey commands are already running. Wait for one to finish, or quit and reopen Dupin to stop them."
            })?;
        let slot = ShellSlot(inner);
        let id = slot.0.next.fetch_add(1, Ordering::Relaxed);
        thread::Builder::new()
            .name(format!("dupin-shell-{id}"))
            .spawn(move || {
                let mut output = ShellOutput::default();
                let status = execute(
                    &slot.0.lifecycle,
                    id,
                    &AtomicBool::new(false),
                    command,
                    &mut Capture::Shell(&mut output),
                    |_| {},
                );
                let closing = lock(&slot.0.lifecycle.state).shutting_down;
                drop(slot);
                complete(if closing { Ok(()) } else { output.result(status) });
                // A descendant can still use stderr after the shell releases its slot.
                if let Some(mut stderr) = output.inherited_stderr
                    && set_nonblocking(stderr.as_raw_fd(), false).is_ok()
                {
                    let _ = io::copy(&mut stderr, &mut io::sink());
                }
            })
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn snapshots(&self) -> Vec<Arc<Snapshot>> {
        lock(&self.inner.runs)
            .iter()
            .map(|run| lock(&run.snapshot).clone())
            .collect()
    }

    pub fn snapshot(&self, id: RunId) -> Option<Arc<Snapshot>> {
        lock(&self.inner.runs)
            .iter()
            .find(|run| run.id == id)
            .map(|run| lock(&run.snapshot).clone())
    }

    pub fn cancel(&self, id: RunId) {
        let run = lock(&self.inner.runs).iter().find(|run| run.id == id).cloned();
        if let Some(run) = run {
            run.cancelled.store(true, Ordering::Release);
            self.inner.lifecycle.cancel(id, false);
        }
    }

    pub fn kill_all_now(&self) {
        self.inner.lifecycle.kill_all_now();
    }
}

struct ShellSlot(Arc<Inner>);

impl Drop for ShellSlot {
    fn drop(&mut self) {
        self.0.running_shells.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.kill_all_now();
    }
}

fn signal_group(pgid: i32, signal: i32) {
    if unsafe { libc::killpg(pgid, signal) } != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            eprintln!("script process group: {error}");
        }
    }
}

fn leader_exited(pid: i32) -> io::Result<bool> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == 0 {
        return Ok(unsafe { info.si_pid() } == pid);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::Interrupted {
        return Ok(false);
    }
    Err(error)
}

fn publish(inner: &Inner, run: &Run, status: Status, output: OutputBuffer) {
    let finished = status != Status::Running;
    let snapshot = Arc::new(Snapshot {
        id: run.id,
        command: run.command.clone(),
        status,
        output,
        elapsed: run.started.elapsed(),
    });
    {
        let mut current = lock(&run.snapshot);
        if finished {
            inner.running_scripts.fetch_sub(1, Ordering::AcqRel);
        }
        *current = snapshot;
    }
    (inner.notify)(run.id, finished);
}

#[derive(Default)]
struct ShellOutput {
    tail: VecDeque<u8>,
    inherited_stderr: Option<ChildStderr>,
}

impl ShellOutput {
    fn feed(&mut self, bytes: &[u8]) {
        let bytes = &bytes[bytes.len().saturating_sub(SHELL_TAIL_BYTES)..];
        let overflow = (self.tail.len() + bytes.len()).saturating_sub(SHELL_TAIL_BYTES);
        self.tail.drain(..overflow);
        self.tail.extend(bytes);
    }

    fn result(&mut self, status: Status) -> Result<(), String> {
        if !status.failed() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(self.tail.make_contiguous());
        let text = text.trim_end();
        let tail = text
            .rmatch_indices('\n')
            .nth(SHELL_TAIL_LINES - 1)
            .map_or(text, |(index, _)| &text[index + 1..]);
        if tail.is_empty() {
            Err(status.to_string())
        } else {
            Err(format!("{status}\n{tail}"))
        }
    }
}

enum Capture<'a> {
    Script(&'a mut OutputBuffer),
    Shell(&'a mut ShellOutput),
}

fn set_nonblocking(fd: RawFd, enabled: bool) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    let flags = if enabled {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct Pipe<R> {
    reader: Option<R>,
    stream: Stream,
    decoder: LineDecoder,
}

impl<R: Read + AsRawFd> Pipe<R> {
    fn new(reader: Option<R>, stream: Stream) -> io::Result<Self> {
        if let Some(reader) = &reader {
            set_nonblocking(reader.as_raw_fd(), true)?;
        }
        Ok(Self {
            reader,
            stream,
            decoder: LineDecoder::default(),
        })
    }

    fn fd(&self) -> RawFd {
        self.reader.as_ref().map_or(-1, AsRawFd::as_raw_fd)
    }

    fn read(&mut self, bytes: &mut [u8], capture: &mut Capture<'_>) -> io::Result<bool> {
        let Some(reader) = &mut self.reader else {
            return Ok(false);
        };
        match reader.read(bytes) {
            Ok(0) => Ok(self.close(capture)),
            Ok(count) => {
                match capture {
                    Capture::Script(output) => self.decoder.feed(&bytes[..count], self.stream, output),
                    Capture::Shell(output) => output.feed(&bytes[..count]),
                }
                Ok(true)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(false)
            }
            Err(error) => {
                self.close(capture);
                Err(error)
            }
        }
    }

    fn close(&mut self, capture: &mut Capture<'_>) -> bool {
        self.reader = None;
        if let Capture::Script(output) = capture
            && self.decoder.has_pending()
        {
            self.decoder.finish(self.stream, output);
            return true;
        }
        false
    }
}

fn execute(
    lifecycle: &Lifecycle,
    id: RunId,
    cancelled: &AtomicBool,
    mut command: Command,
    capture: &mut Capture<'_>,
    mut notify: impl FnMut(&OutputBuffer),
) -> Status {
    let stdout = match capture {
        Capture::Script(_) => Stdio::piped(),
        Capture::Shell(_) => Stdio::null(),
    };
    command
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = match lifecycle.spawn(id, cancelled, || {
        command.spawn().map(|child| {
            let pgid = child.id() as i32;
            (child, pgid)
        })
    }) {
        Ok(Some(child)) => child,
        Ok(None) => return Status::Cancelled,
        Err(error) => return Status::Failed(error.to_string()),
    };
    let pipes = Pipe::new(child.stdout.take(), Stream::Stdout)
        .and_then(|stdout| Pipe::new(child.stderr.take(), Stream::Stderr).map(|stderr| (stdout, stderr)));
    let (mut stdout, mut stderr) = match pipes {
        Ok(pipes) => pipes,
        Err(error) => {
            lifecycle.cancel(id, true);
            let _ = child.wait();
            lifecycle.reaped(id);
            return Status::Failed(error.to_string());
        }
    };
    let mut bytes = [0; 8192];
    let mut problem = None;
    let mut cancel_at = None;
    let mut exited = None;
    let mut dirty = false;
    let mut notified = Instant::now();
    loop {
        if exited.is_none() {
            if cancelled.load(Ordering::Acquire) {
                let started = cancel_at.get_or_insert_with(Instant::now);
                let force = started.elapsed() >= CANCEL_GRACE;
                lifecycle.cancel(id, force);
            }
            match lifecycle.reap(id, leader_exited, || child.try_wait()) {
                Ok(Some((status, cancelled))) => {
                    if let Capture::Shell(output) = capture {
                        if !status.success()
                            && let Some(stderr) = &mut stderr.reader
                        {
                            // A descendant writing continuously mustn't delay the failure notice.
                            let mut pending = stderr.take(256 * 1024);
                            while let Ok(count @ 1..) = pending.read(&mut bytes) {
                                output.feed(&bytes[..count]);
                            }
                        }
                        output.inherited_stderr = stderr.reader.take();
                        return problem.map_or_else(|| Status::Exited(status.code()), Status::Failed);
                    }
                    exited = Some((status, cancelled, Instant::now()));
                }
                Ok(None) => {}
                Err(error) => {
                    lifecycle.cancel(id, true);
                    let _ = child.wait();
                    lifecycle.reaped(id);
                    return Status::Failed(error.to_string());
                }
            }
        }
        let mut timeout_ms = POLL_INTERVAL_MS;
        if let Some((status, cancelled, at)) = exited {
            let remaining = EXIT_DRAIN.saturating_sub(at.elapsed());
            if (stdout.fd() == -1 && stderr.fd() == -1) || remaining.is_zero() {
                stdout.close(capture);
                stderr.close(capture);
                return if cancelled {
                    Status::Cancelled
                } else if let Some(error) = problem {
                    Status::Failed(error)
                } else {
                    Status::Exited(status.code())
                };
            }
            timeout_ms = timeout_ms.min(remaining.as_millis() as i32);
        }
        let mut fds = [stdout.fd(), stderr.fd()].map(|fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        });
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) } == -1 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                lifecycle.cancel(id, true);
                let _ = child.wait();
                lifecycle.reaped(id);
                return Status::Failed(error.to_string());
            }
        }
        if fds.iter().any(|fd| fd.revents != 0) {
            let reads = [stdout.read(&mut bytes, capture), stderr.read(&mut bytes, capture)];
            for result in reads {
                match result {
                    Ok(changed) => dirty |= changed,
                    Err(error) => problem = Some(error.to_string()),
                }
            }
        }
        if dirty
            && notified.elapsed() >= Duration::from_millis(50)
            && let Capture::Script(output) = capture
        {
            notify(output);
            notified = Instant::now();
            dirty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_core::script::OutputMode;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::{Barrier, mpsc},
    };

    type RecordedSignals = Arc<Mutex<Vec<(i32, i32)>>>;
    fn lifecycle() -> (Arc<Lifecycle>, RecordedSignals) {
        let signals = Arc::new(Mutex::new(Vec::new()));
        let captured = signals.clone();
        let lifecycle = Arc::new(Lifecycle {
            state: Mutex::default(),
            signal: Arc::new(move |pgid, signal| lock(&captured).push((pgid, signal))),
        });
        (lifecycle, signals)
    }

    #[test]
    fn shutdown_serializes_with_spawn_and_refuses_later_spawns() {
        let (lifecycle, signals) = lifecycle();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let worker = {
            let lifecycle = lifecycle.clone();
            let entered = entered.clone();
            let release = release.clone();
            thread::spawn(move || {
                lifecycle
                    .spawn(1, &AtomicBool::new(false), || {
                        entered.wait();
                        release.wait();
                        Ok(((), 123))
                    })
                    .unwrap();
            })
        };
        entered.wait();
        assert!(lifecycle.state.try_lock().is_err());
        let (done, received) = mpsc::channel();
        let shutdown = {
            let lifecycle = lifecycle.clone();
            thread::spawn(move || {
                lifecycle.kill_all_now();
                done.send(()).unwrap();
            })
        };
        assert!(received.try_recv().is_err());
        release.wait();
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        shutdown.join().unwrap();
        lifecycle.kill_all_now();
        assert_eq!(*lock(&signals), [(123, libc::SIGKILL)]);
        assert_eq!(
            lifecycle
                .spawn::<()>(2, &AtomicBool::new(false), || panic!("spawn after quit"))
                .unwrap(),
            None
        );
    }

    #[test]
    fn cancellation_does_not_hold_the_lock_between_signals() {
        let (lifecycle, signals) = lifecycle();
        lifecycle
            .spawn(1, &AtomicBool::new(false), || Ok(((), 456)))
            .unwrap();
        let (waiting, received) = mpsc::channel();
        let release = Arc::new(Barrier::new(2));
        let cancellation = {
            let lifecycle = lifecycle.clone();
            let release = release.clone();
            thread::spawn(move || {
                lifecycle.cancel(1, false);
                waiting.send(()).unwrap();
                release.wait();
                lifecycle.cancel(1, true);
            })
        };
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        lifecycle.kill_all_now();
        release.wait();
        cancellation.join().unwrap();
        assert_eq!(*lock(&signals), [(456, libc::SIGTERM), (456, libc::SIGKILL)]);
        lifecycle.reaped(1);
        lifecycle.kill_all_now();
        assert_eq!(lock(&signals).len(), 2);
    }

    #[test]
    fn failed_and_cancelled_spawns_register_no_group() {
        let (lifecycle, signals) = lifecycle();
        assert!(
            lifecycle
                .spawn::<()>(1, &AtomicBool::new(false), || Err(io::Error::other("fixture")))
                .is_err()
        );
        assert_eq!(
            lifecycle
                .spawn::<()>(2, &AtomicBool::new(true), || panic!("cancelled spawn"))
                .unwrap(),
            None
        );
        lifecycle.kill_all_now();
        assert!(lock(&signals).is_empty());
    }

    #[test]
    fn reaping_reserves_the_group_until_removal() {
        let (lifecycle, signals) = lifecycle();
        lifecycle
            .spawn(1, &AtomicBool::new(false), || Ok(((), 789)))
            .unwrap();
        lifecycle
            .reap(
                1,
                |_| panic!("observing exit of a run that was not cancelled"),
                || {
                    assert!(lifecycle.state.try_lock().is_err());
                    Ok(Some(()))
                },
            )
            .unwrap();
        lifecycle.cancel(1, false);
        lifecycle.kill_all_now();
        assert!(lock(&signals).is_empty());
    }

    #[test]
    fn cancellation_reserves_the_group_until_escalation() {
        let (lifecycle, signals) = lifecycle();
        lifecycle
            .spawn(1, &AtomicBool::new(false), || Ok(((), 789)))
            .unwrap();
        lifecycle.cancel(1, false);
        assert_eq!(
            lifecycle
                .reap::<()>(1, |_| Ok(false), || panic!("reaping a live leader after TERM"))
                .unwrap(),
            None
        );
        lifecycle.cancel(1, true);
        assert_eq!(
            lifecycle
                .reap(1, |_| panic!("observing exit after KILL"), || Ok(Some(())))
                .unwrap(),
            Some(((), true))
        );
        assert_eq!(*lock(&signals), [(789, libc::SIGTERM), (789, libc::SIGKILL)]);
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(body: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "dupin-runner-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::write(path.join("run"), format!("#!/bin/sh\n{body}")).unwrap();
            fs::set_permissions(path.join("run"), fs::Permissions::from_mode(0o700)).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }

        fn command(&self, args: &[&str]) -> ScriptRun {
            ScriptRun {
                path: self.0.join("run"),
                title: "Fixture".into(),
                args: args.iter().map(|arg| (*arg).into()).collect(),
                mode: OutputMode::Inline,
            }
        }

        fn start_shell(&self, runner: &Runner) -> mpsc::Receiver<Result<(), String>> {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "./run"]).current_dir(&self.0);
            let (send, received) = mpsc::channel();
            runner
                .start_shell_command(command, move |result| send.send(result).unwrap())
                .unwrap();
            received
        }

        fn wait_for(&self, file: &str) {
            let started = Instant::now();
            while !self.0.join(file).exists() {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "{file} was not created"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn wait(runner: &Runner, id: RunId) -> Arc<Snapshot> {
        let start = Instant::now();
        loop {
            let snapshot = runner.snapshot(id).unwrap();
            if snapshot.status != Status::Running {
                return snapshot;
            }
            assert!(start.elapsed() < Duration::from_secs(6), "script did not finish");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn shell_notices_collect_only_the_bounded_stderr_tail() {
        let fixture = Fixture::new(
            "read value && exit 99\nprintf 'stdout\\n'\nprintf '%s\\n' old first second third fourth >&2\nexit 7\n",
        );
        let runner = Runner::new(
            fixture.0.clone(),
            Arc::new(|_, _| panic!("shell in script history")),
        );
        assert_eq!(
            fixture
                .start_shell(&runner)
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            Err("Exited with code 7\nfirst\nsecond\nthird\nfourth".into())
        );
        let long_line = Fixture::new("printf '%02000d' 0 >&2\nexit 7\n");
        assert_eq!(
            long_line
                .start_shell(&runner)
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            Err(format!("Exited with code 7\n{}", "0".repeat(SHELL_TAIL_BYTES)))
        );
        assert!(runner.snapshots().is_empty());
        assert_eq!(runner.activity().load(Ordering::Acquire), 0);
    }

    #[test]
    fn shell_releases_its_slot_before_inheriting_children_finish_writing() {
        let fixture = Fixture::new(
            "(i=0; until [ -f release ]; do i=$((i + 1)); [ \"$i\" -lt 100 ] || exit 1; sleep 0.05; done;\n\
            printf 'late stderr\\n' >&2 && touch alive) &\nexit 0\n",
        );
        let runner = Runner::new(fixture.0.clone(), Arc::new(|_, _| {}));
        assert_eq!(
            fixture
                .start_shell(&runner)
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            Ok(())
        );
        assert_eq!(runner.inner.running_shells.load(Ordering::Acquire), 0);
        fs::write(fixture.0.join("release"), "").unwrap();
        fixture.wait_for("alive");
    }

    #[test]
    fn shutdown_stops_an_active_shell_group_and_prevents_later_launches() {
        let fixture = Fixture::new("sleep 30 &\ntouch ready\nwait\n");
        let runner = Runner::new(fixture.0.clone(), Arc::new(|_, _| {}));
        let completion = fixture.start_shell(&runner);
        fixture.wait_for("ready");
        assert_eq!(runner.inner.running_shells.load(Ordering::Acquire), 1);
        assert_eq!(runner.activity().load(Ordering::Acquire), 0);
        let pgid = lock(&runner.inner.lifecycle.state).groups[&1].pgid;
        runner.kill_all_now();
        assert_eq!(completion.recv_timeout(Duration::from_secs(2)).unwrap(), Ok(()));
        let refused = Fixture::new("touch launched\n");
        assert_eq!(
            refused
                .start_shell(&runner)
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            Ok(())
        );
        assert!(!refused.0.join("launched").exists());
        let deadline = Instant::now();
        while unsafe { libc::killpg(pgid, 0) } == 0 {
            assert!(
                deadline.elapsed() < Duration::from_secs(2),
                "shell group survived quit"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn direct_execution_preserves_argv_closes_stdin_and_collects_failure() {
        let fixture = Fixture::new(
            "read value && exit 99\nprintf '%s\\n' \"$PWD\" \"$1\" \"$2\"\nprintf 'fixture error' >&2\nexit 7\n",
        );
        let runner = Runner::new(fixture.0.clone(), Arc::new(|_, _| {}));
        let running = runner.activity().clone();
        let id = runner.start(fixture.command(&["$HOME * ~", "a b"])).id;
        let result = wait(&runner, id);
        assert_eq!(result.status, Status::Exited(Some(7)));
        let stdout: Vec<_> = result
            .output
            .lines()
            .iter()
            .filter(|line| line.stream == Stream::Stdout)
            .map(|line| line.text.as_ref())
            .collect();
        assert_eq!(stdout, [fixture.0.to_str().unwrap(), "$HOME * ~", "a b"]);
        assert_eq!(result.output.stderr_tail(), "fixture error\n");
        assert_eq!(running.load(Ordering::Acquire), 0);
    }

    #[test]
    fn leader_exit_finishes_even_with_inherited_pipes() {
        let fixture = Fixture::new(
            "(for _ in 1 2 3 4 5 6 7 8; do if [ -f stop ]; then touch stopped; exit; fi; sleep 0.25; done; touch timed-out) &\nprintf 'last stdout'\nprintf 'last stderr' >&2\nexit 7\n",
        );
        let runner = Runner::new(fixture.0.clone(), Arc::new(|_, _| {}));
        let running = runner.activity().clone();
        let id = runner.start(fixture.command(&[])).id;
        let result = wait(&runner, id);
        fs::write(fixture.0.join("stop"), "").unwrap();
        assert!(!fixture.0.join("timed-out").exists());
        assert_eq!(result.status, Status::Exited(Some(7)));
        assert_eq!(result.output.last_stdout(), Some("last stdout"));
        assert_eq!(result.output.stderr_tail(), "last stderr\n");
        assert_eq!(running.load(Ordering::Acquire), 0);
        let start = Instant::now();
        while !fixture.0.join("stopped").exists() {
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn cancellation_finishes_promptly_when_the_leader_exits_on_term() {
        let fixture = Fixture::new(
            "trap 'exit 0' TERM\n(sh -c 'trap \"\" TERM; printf \"ready\\n\"; exec sleep 60') &\nwait\n",
        );
        let runner = Runner::new(fixture.0.clone(), Arc::new(|_, _| {}));
        let id = runner.start(fixture.command(&[])).id;
        let start = Instant::now();
        while runner.snapshot(id).unwrap().output.last_stdout() != Some("ready") {
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(10));
        }
        let pgid = lock(&runner.inner.lifecycle.state).groups[&id].pgid;
        let cancelled = Instant::now();
        runner.cancel(id);
        assert_eq!(wait(&runner, id).status, Status::Cancelled);
        assert_eq!(runner.activity().load(Ordering::Acquire), 0);
        assert!(cancelled.elapsed() < CANCEL_GRACE);
        let deadline = Instant::now();
        while unsafe { libc::killpg(pgid, 0) } == 0 {
            assert!(
                deadline.elapsed() < Duration::from_secs(2),
                "fixture group survived"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn cancellation_escalates_for_a_term_ignoring_process_group() {
        let fixture = Fixture::new("trap '' TERM\nsleep 60 &\nprintf 'ready\\n'\nwait\n");
        let runner = Runner::new(fixture.0.clone(), Arc::new(|_, _| {}));
        let id = runner.start(fixture.command(&[])).id;
        let start = Instant::now();
        while runner.snapshots()[0].output.last_stdout() != Some("ready") {
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(10));
        }
        let pgid = lock(&runner.inner.lifecycle.state).groups[&id].pgid;
        let cancelled = Instant::now();
        runner.cancel(id);
        assert_eq!(wait(&runner, id).status, Status::Cancelled);
        assert!(cancelled.elapsed() >= Duration::from_secs(2));
        let deadline = Instant::now();
        while unsafe { libc::killpg(pgid, 0) } == 0 {
            assert!(
                deadline.elapsed() < Duration::from_secs(2),
                "fixture group survived cancellation"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn active_limit_and_drop_cleanup_include_queued_runs() {
        let fixture = Fixture::new("sleep 60\n");
        let runner = Runner::new(fixture.0.clone(), Arc::new(|_, _| {}));
        let running = runner.activity().clone();
        for _ in 0..8 {
            assert!(!runner.start(fixture.command(&[])).refused);
        }
        let mut silent = fixture.command(&[]);
        silent.mode = OutputMode::Silent;
        let start = runner.start(silent.clone());
        assert!(start.refused);
        let failure = runner.snapshot(start.id).unwrap();
        assert_eq!(*failure.command, silent);
        assert_eq!(
            failure.status,
            Status::Failed("Eight scripts are already running.".into())
        );
        assert_eq!(runner.snapshots()[0].id, failure.id);
        assert_eq!(running.load(Ordering::Acquire), 8);
        let inner = runner.inner.clone();
        drop(runner);
        let start = Instant::now();
        while running.load(Ordering::Acquire) != 0 {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "drop left a running fixture"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(lock(&inner.lifecycle.state).groups.is_empty());
    }

    #[test]
    #[ignore = "subprocess fixture for cancellation_keeps_a_slot_from_waiting_on_escaped_pipes"]
    fn escaped_pipe_holder() {
        let Some(marker) = std::env::var_os("DUPIN_RUNNER_ESCAPE_MARKER") else {
            return;
        };
        assert!(unsafe { libc::setsid() } >= 0);
        let marker = PathBuf::from(marker);
        fs::write(&marker, "ready").unwrap();
        let start = Instant::now();
        while !marker.with_extension("stop").exists() && start.elapsed() < Duration::from_secs(6) {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::write(marker.with_extension("done"), "done");
    }

    #[test]
    fn cancellation_keeps_a_slot_from_waiting_on_escaped_pipes() {
        let fixture = Fixture::new(
            "DUPIN_RUNNER_ESCAPE_MARKER=\"$2\" \"$1\" --exact scripts::runner::tests::escaped_pipe_holder --ignored --nocapture &\nwait\n",
        );
        let runner = Runner::new(fixture.0.clone(), Arc::new(|_, _| {}));
        let running = runner.activity().clone();
        let executable = std::env::current_exe().unwrap();
        let marker = fixture.0.join("holder.ready");
        let id = runner
            .start(fixture.command(&[executable.to_str().unwrap(), marker.to_str().unwrap()]))
            .id;
        let start = Instant::now();
        while !marker.exists() {
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(10));
        }
        runner.cancel(id);
        assert_eq!(wait(&runner, id).status, Status::Cancelled);
        assert_eq!(running.load(Ordering::Acquire), 0);
        assert!(!marker.with_extension("done").exists());
        fs::write(marker.with_extension("stop"), "stop").unwrap();
        let start = Instant::now();
        while !marker.with_extension("done").exists() {
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(10));
        }
    }
}
