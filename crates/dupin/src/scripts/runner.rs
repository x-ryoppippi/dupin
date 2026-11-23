use dupin_core::script::{LineDecoder, OutputBuffer, ScriptRun, Stream};
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Read},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub type RunId = u64;
type Notify = Arc<dyn Fn(RunId) + Send + Sync>;
type Signal = Arc<dyn Fn(i32, i32) + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Exited(Option<i32>),
    Cancelled,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub id: RunId,
    pub command: ScriptRun,
    pub status: Status,
    pub output: OutputBuffer,
    pub elapsed: Duration,
}

struct Run {
    id: RunId,
    command: ScriptRun,
    started: Instant,
    cancelled: AtomicBool,
    data: Mutex<RunData>,
}

struct RunData {
    status: Status,
    output: OutputBuffer,
    elapsed: Option<Duration>,
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

    fn reap<T>(&self, id: RunId, poll: impl FnOnce() -> io::Result<Option<T>>) -> io::Result<Option<T>> {
        let mut state = lock(&self.state);
        let result = poll()?;
        if result.is_some() {
            state.groups.remove(&id);
        }
        Ok(result)
    }

    fn killed(&self, id: RunId) -> bool {
        lock(&self.state)
            .groups
            .get(&id)
            .is_some_and(|group| group.killed)
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
    pub fn new(home: PathBuf, running: Arc<AtomicUsize>, notify: Notify) -> Self {
        Self {
            inner: Arc::new(Inner {
                lifecycle: Lifecycle {
                    state: Mutex::default(),
                    signal: Arc::new(signal_group),
                },
                runs: Mutex::default(),
                next: AtomicU64::new(1),
                running,
                notify,
                home,
            }),
        }
    }

    pub fn start(&self, command: ScriptRun) -> Result<RunId, String> {
        let mut runs = lock(&self.inner.runs);
        if lock(&self.inner.lifecycle.state).shutting_down {
            return Err("Dupin is shutting down.".into());
        }
        if runs
            .iter()
            .filter(|run| lock(&run.data).status == Status::Running)
            .count()
            >= 8
        {
            return Err("Eight scripts are already running.".into());
        }
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let run = Arc::new(Run {
            id,
            command,
            started: Instant::now(),
            cancelled: AtomicBool::new(false),
            data: Mutex::new(RunData {
                status: Status::Running,
                output: OutputBuffer::default(),
                elapsed: None,
            }),
        });
        while runs.len() >= 20 {
            let index = runs
                .iter()
                .position(|run| lock(&run.data).status != Status::Running)
                .unwrap();
            runs.remove(index);
        }
        runs.push_front(run.clone());
        drop(runs);
        self.inner.running.fetch_add(1, Ordering::AcqRel);
        let inner = self.inner.clone();
        let worker_run = run.clone();
        if let Err(error) = thread::Builder::new()
            .name(format!("dupin-script-{id}"))
            .spawn(move || {
                let status = execute(&inner, &worker_run);
                finish(&inner, &worker_run, status);
            })
        {
            finish(&self.inner, &run, Status::Failed(error.to_string()));
        }
        Ok(id)
    }

    pub fn snapshots(&self) -> Vec<Snapshot> {
        lock(&self.inner.runs)
            .iter()
            .map(|run| {
                let data = lock(&run.data);
                Snapshot {
                    id: run.id,
                    command: run.command.clone(),
                    status: data.status.clone(),
                    output: data.output.clone(),
                    elapsed: data.elapsed.unwrap_or_else(|| run.started.elapsed()),
                }
            })
            .collect()
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

fn finish(inner: &Inner, run: &Run, status: Status) {
    {
        let mut data = lock(&run.data);
        data.status = status;
        data.elapsed = Some(run.started.elapsed());
    }
    (inner.notify)(run.id);
    inner.running.fetch_sub(1, Ordering::AcqRel);
}

enum Output {
    Bytes(Stream, Vec<u8>),
    End(Stream, io::Result<()>),
}

fn reader(
    mut pipe: impl Read + Send + 'static,
    stream: Stream,
    tx: mpsc::SyncSender<Output>,
) -> io::Result<()> {
    thread::Builder::new()
        .name("dupin-script-output".into())
        .spawn(move || {
            let result = loop {
                let mut bytes = vec![0; 8192];
                match pipe.read(&mut bytes) {
                    Ok(0) => break Ok(()),
                    Ok(count) => {
                        bytes.truncate(count);
                        if tx.send(Output::Bytes(stream, bytes)).is_err() {
                            return;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => break Err(error),
                }
            };
            let _ = tx.send(Output::End(stream, result));
        })
        .map(|_| ())
}

fn execute(inner: &Inner, run: &Run) -> Status {
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
    let (tx, rx) = mpsc::sync_channel(16);
    let readers = reader(child.stdout.take().unwrap(), Stream::Stdout, tx.clone())
        .and_then(|()| reader(child.stderr.take().unwrap(), Stream::Stderr, tx));
    if let Err(error) = readers {
        inner.lifecycle.cancel(run.id, true);
        let _ = child.wait();
        inner.lifecycle.reaped(run.id);
        return Status::Failed(error.to_string());
    }
    let mut decoders = [LineDecoder::default(), LineDecoder::default()];
    let mut ended = 0;
    let mut problem = None;
    let mut cancel_at = None;
    let mut dirty = false;
    let mut notified = Instant::now();
    loop {
        if run.cancelled.load(Ordering::Acquire) {
            let started = cancel_at.get_or_insert_with(Instant::now);
            inner
                .lifecycle
                .cancel(run.id, started.elapsed() >= Duration::from_secs(2));
        }
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Output::Bytes(stream, bytes)) => {
                let index = usize::from(stream == Stream::Stderr);
                decoders[index].feed(&bytes, stream, &mut lock(&run.data).output);
                dirty = true;
            }
            Ok(Output::End(stream, result)) => {
                let decoder = &mut decoders[usize::from(stream == Stream::Stderr)];
                if decoder.has_pending() {
                    decoder.finish(stream, &mut lock(&run.data).output);
                }
                if let Err(error) = result {
                    problem = Some(error.to_string());
                }
                ended += 1;
                dirty = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => thread::sleep(Duration::from_millis(20)),
        }
        if dirty && notified.elapsed() >= Duration::from_millis(50) {
            (inner.notify)(run.id);
            notified = Instant::now();
            dirty = false;
        }
        if ended == 2 || inner.lifecycle.killed(run.id) {
            match inner.lifecycle.reap(run.id, || child.try_wait()) {
                Ok(Some(status)) => {
                    return if run.cancelled.load(Ordering::Acquire) {
                        Status::Cancelled
                    } else if let Some(error) = problem {
                        Status::Failed(error)
                    } else {
                        Status::Exited(status.code())
                    };
                }
                Ok(None) => {}
                Err(error) => {
                    inner.lifecycle.cancel(run.id, true);
                    let _ = child.wait();
                    inner.lifecycle.reaped(run.id);
                    return Status::Failed(error.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_core::script::OutputMode;
    use std::{fs, os::unix::fs::PermissionsExt, sync::Barrier};

    fn lifecycle() -> (Arc<Lifecycle>, Arc<Mutex<Vec<(i32, i32)>>>) {
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
            .reap(1, || {
                assert!(lifecycle.state.try_lock().is_err());
                Ok(Some(()))
            })
            .unwrap();
        lifecycle.cancel(1, false);
        lifecycle.kill_all_now();
        assert!(lock(&signals).is_empty());
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

    fn wait(runner: &Runner, id: RunId) -> Snapshot {
        let start = Instant::now();
        loop {
            let snapshot = runner.snapshots().into_iter().find(|run| run.id == id).unwrap();
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
        let running = Arc::new(AtomicUsize::new(0));
        let runner = Runner::new(fixture.0.clone(), running.clone(), Arc::new(|_| {}));
        let id = runner.start(fixture.command(&["$HOME * ~", "a b"])).unwrap();
        let result = wait(&runner, id);
        assert_eq!(result.status, Status::Exited(Some(7)));
        let stdout: Vec<_> = result
            .output
            .lines()
            .iter()
            .filter(|line| line.stream == Stream::Stdout)
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(stdout, [fixture.0.to_str().unwrap(), "$HOME * ~", "a b"]);
        assert_eq!(result.output.stderr_tail(), "fixture error\n");
        assert_eq!(running.load(Ordering::Acquire), 0);
    }

    #[test]
    fn cancellation_escalates_for_a_term_ignoring_process_group() {
        let fixture = Fixture::new("trap '' TERM\nsleep 60 &\nprintf 'ready\\n'\nwait\n");
        let runner = Runner::new(fixture.0.clone(), Arc::new(AtomicUsize::new(0)), Arc::new(|_| {}));
        let id = runner.start(fixture.command(&[])).unwrap();
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
        let running = Arc::new(AtomicUsize::new(0));
        let runner = Runner::new(fixture.0.clone(), running.clone(), Arc::new(|_| {}));
        for _ in 0..8 {
            runner.start(fixture.command(&[])).unwrap();
        }
        assert!(runner.start(fixture.command(&[])).unwrap_err().contains("Eight"));
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
        let running = Arc::new(AtomicUsize::new(0));
        let runner = Runner::new(fixture.0.clone(), running.clone(), Arc::new(|_| {}));
        let executable = std::env::current_exe().unwrap();
        let marker = fixture.0.join("holder.ready");
        let id = runner
            .start(fixture.command(&[executable.to_str().unwrap(), marker.to_str().unwrap()]))
            .unwrap();
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
