use dupin_core::script::{LineDecoder, OutputBuffer, ScriptRun, Stream};
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Read},
    os::fd::{AsRawFd, RawFd},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
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
    running: Arc<AtomicUsize>,
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
                running: Arc::default(),
                notify,
                home,
            }),
        }
    }

    pub fn activity(&self) -> &Arc<AtomicUsize> {
        &self.inner.running
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
        self.inner.running.fetch_add(1, Ordering::AcqRel);
        let inner = self.inner.clone();
        let worker_run = run.clone();
        if let Err(error) = thread::Builder::new()
            .name(format!("dupin-script-{id}"))
            .spawn(move || {
                let mut output = OutputBuffer::default();
                let status = execute(&inner, &worker_run, &mut output);
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
            inner.running.fetch_sub(1, Ordering::AcqRel);
        }
        *current = snapshot;
    }
    (inner.notify)(run.id, finished);
}

struct Pipe<R> {
    reader: Option<R>,
    stream: Stream,
    decoder: LineDecoder,
}

impl<R: Read + AsRawFd> Pipe<R> {
    fn new(reader: R, stream: Stream) -> io::Result<Self> {
        let fd = reader.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            reader: Some(reader),
            stream,
            decoder: LineDecoder::default(),
        })
    }

    fn fd(&self) -> RawFd {
        self.reader.as_ref().map_or(-1, AsRawFd::as_raw_fd)
    }

    fn read(&mut self, bytes: &mut [u8], output: &mut OutputBuffer) -> io::Result<bool> {
        let Some(reader) = &mut self.reader else {
            return Ok(false);
        };
        match reader.read(bytes) {
            Ok(0) => Ok(self.close(output)),
            Ok(count) => {
                self.decoder.feed(&bytes[..count], self.stream, output);
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
                self.close(output);
                Err(error)
            }
        }
    }

    fn close(&mut self, output: &mut OutputBuffer) -> bool {
        self.reader = None;
        if self.decoder.has_pending() {
            self.decoder.finish(self.stream, output);
            return true;
        }
        false
    }
}

fn execute(inner: &Inner, run: &Run, output: &mut OutputBuffer) -> Status {
    let mut command = Command::new(&run.command.path);
    command
        .args(&run.command.args)
        .current_dir(&inner.home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = match inner.lifecycle.spawn(run.id, &run.cancelled, || {
        command.spawn().map(|child| {
            let pgid = child.id() as i32;
            (child, pgid)
        })
    }) {
        Ok(Some(child)) => child,
        Ok(None) => return Status::Cancelled,
        Err(error) => return Status::Failed(error.to_string()),
    };
    let pipes = Pipe::new(child.stdout.take().unwrap(), Stream::Stdout).and_then(|stdout| {
        Pipe::new(child.stderr.take().unwrap(), Stream::Stderr).map(|stderr| (stdout, stderr))
    });
    let (mut stdout, mut stderr) = match pipes {
        Ok(pipes) => pipes,
        Err(error) => {
            inner.lifecycle.cancel(run.id, true);
            let _ = child.wait();
            inner.lifecycle.reaped(run.id);
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
            if run.cancelled.load(Ordering::Acquire) {
                let started = cancel_at.get_or_insert_with(Instant::now);
                let force = started.elapsed() >= CANCEL_GRACE;
                inner.lifecycle.cancel(run.id, force);
            }
            match inner.lifecycle.reap(run.id, leader_exited, || child.try_wait()) {
                Ok(Some((status, cancelled))) => exited = Some((status, cancelled, Instant::now())),
                Ok(None) => {}
                Err(error) => {
                    inner.lifecycle.cancel(run.id, true);
                    let _ = child.wait();
                    inner.lifecycle.reaped(run.id);
                    return Status::Failed(error.to_string());
                }
            }
        }
        let mut timeout_ms = POLL_INTERVAL_MS;
        if let Some((status, cancelled, at)) = exited {
            let remaining = EXIT_DRAIN.saturating_sub(at.elapsed());
            if (stdout.fd() == -1 && stderr.fd() == -1) || remaining.is_zero() {
                stdout.close(output);
                stderr.close(output);
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
                inner.lifecycle.cancel(run.id, true);
                let _ = child.wait();
                inner.lifecycle.reaped(run.id);
                return Status::Failed(error.to_string());
            }
        }
        if fds.iter().any(|fd| fd.revents != 0) {
            let reads = [stdout.read(&mut bytes, output), stderr.read(&mut bytes, output)];
            for result in reads {
                match result {
                    Ok(changed) => dirty |= changed,
                    Err(error) => problem = Some(error.to_string()),
                }
            }
        }
        if dirty && notified.elapsed() >= Duration::from_millis(50) {
            publish(inner, run, Status::Running, output.clone());
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
