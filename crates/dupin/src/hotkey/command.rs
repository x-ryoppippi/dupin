use std::{
    collections::{HashSet, VecDeque},
    env,
    io::{self, Read},
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

const STDERR_BYTES: usize = 512;
const STDERR_LINES: usize = 4;
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const TIME_LIMIT: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
pub struct Runner {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    closing: bool,
    groups: HashSet<u32>,
}

impl Runner {
    pub fn run(&self, line: &str, complete: impl FnOnce(Result<(), String>)) {
        let Some(home) = env::home_dir() else {
            complete(Err("Couldn't determine the home directory.".into()));
            return;
        };
        let shell = env::var_os("SHELL")
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "/bin/zsh".into());
        let mut command = Command::new(shell);
        command.args(["-lc", line]).current_dir(home);
        self.execute(command, TIME_LIMIT, complete);
    }

    pub fn stop(&self) {
        let mut state = self.lock();
        state.closing = true;
        for group in state.groups.drain() {
            kill_group(group);
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn spawn(&self, mut command: Command) -> io::Result<Option<(Child, UnixStream)>> {
        let (stderr, writer) = UnixStream::pair()?;
        // Quit must see every spawned group, including one whose worker was just scheduled.
        let mut state = self.lock();
        if state.closing {
            return Ok(None);
        }
        let child = command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(OwnedFd::from(writer))
            .spawn()?;
        state.groups.insert(child.id());
        Ok(Some((child, stderr)))
    }

    fn execute(&self, command: Command, time_limit: Duration, complete: impl FnOnce(Result<(), String>)) {
        let (mut child, mut stderr) = match self.spawn(command) {
            Ok(Some(run)) => run,
            Ok(None) => return complete(Ok(())),
            Err(error) => return complete(Err(error.to_string())),
        };
        let mut tail = VecDeque::with_capacity(STDERR_BYTES);
        match self.collect(&mut child, &mut stderr, &mut tail, time_limit) {
            Ok(status) => {
                let closing = self.lock().closing;
                complete(if closing { Ok(()) } else { result(status, tail) });
                // A descendant can still be using stderr. Release the command slot but
                // keep draining here so a write after the shell exits doesn't raise SIGPIPE.
                if stderr.set_read_timeout(None).is_ok() && stderr.set_nonblocking(false).is_ok() {
                    let _ = io::copy(&mut stderr, &mut io::sink());
                }
            }
            Err(error) => {
                {
                    let mut state = self.lock();
                    if state.groups.remove(&child.id()) {
                        kill_group(child.id());
                    }
                }
                let _ = child.wait();
                complete(Err(error.to_string()));
            }
        }
    }

    fn collect(
        &self,
        child: &mut Child,
        stderr: &mut UnixStream,
        tail: &mut VecDeque<u8>,
        time_limit: Duration,
    ) -> io::Result<ExitStatus> {
        stderr.set_read_timeout(Some(POLL_INTERVAL))?;
        let started = Instant::now();
        let mut bytes = [0; 8192];
        let mut eof = false;
        loop {
            let status = {
                // Quit must never signal a group after the shell PID has been reaped and reused.
                let mut state = self.lock();
                let status = child.try_wait()?;
                if status.is_some() {
                    state.groups.remove(&child.id());
                }
                status
            };
            if let Some(status) = status {
                if !status.success() && !eof && stderr.set_nonblocking(true).is_ok() {
                    // A descendant writing continuously mustn't delay the failure notice.
                    let mut pending = stderr.take(256 * 1024);
                    while let Ok(count @ 1..) = pending.read(&mut bytes) {
                        keep_tail(tail, &bytes[..count]);
                    }
                }
                return Ok(status);
            }
            if started.elapsed() >= time_limit {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "Command timed out."));
            }
            if eof {
                thread::sleep(POLL_INTERVAL);
                continue;
            }
            match stderr.read(&mut bytes) {
                Ok(0) => eof = true,
                Ok(count) => keep_tail(tail, &bytes[..count]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

fn kill_group(group: u32) {
    // SAFETY: The group ID comes from a child spawned with process_group(0).
    unsafe { libc::killpg(group as libc::pid_t, libc::SIGKILL) };
}

fn result(status: ExitStatus, mut tail: VecDeque<u8>) -> Result<(), String> {
    if status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(tail.make_contiguous());
    let text = text.trim_end();
    let tail = text
        .rmatch_indices('\n')
        .nth(STDERR_LINES - 1)
        .map_or(text, |(index, _)| &text[index + 1..]);
    if tail.is_empty() {
        Err(status.to_string())
    } else {
        Err(format!("{status}\n{tail}"))
    }
}

fn keep_tail(tail: &mut VecDeque<u8>, bytes: &[u8]) {
    let bytes = &bytes[bytes.len().saturating_sub(STDERR_BYTES)..];
    let overflow = (tail.len() + bytes.len()).saturating_sub(STDERR_BYTES);
    tail.drain(..overflow);
    tail.extend(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};

    fn fixture(line: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", line]);
        command
    }

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = env::temp_dir().join(format!("dupin-command-{name}-{}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn wait_for(path: &std::path::Path) {
        let start = std::time::Instant::now();
        while !path.exists() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "{} was not created",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn background_writer(directory: &TestDirectory) -> Command {
        let mut command = fixture(
            r#"(i=0
                until [ -e release ]; do
                    i=$((i + 1))
                    [ "$i" -lt 500 ] || exit 1
                    sleep 0.02
                done
                printf '%s\n' 'late stderr' >&2 && touch alive
            ) &"#,
        );
        command.current_dir(&directory.0);
        command
    }

    #[test]
    fn a_background_descendant_can_write_stderr_after_the_shell_has_exited() {
        let directory = TestDirectory::new("background");
        let command = background_writer(&directory);
        let (completed, result) = mpsc::channel();
        let worker = thread::spawn(move || {
            Runner::default().execute(command, TIME_LIMIT, |result| completed.send(result).unwrap());
        });
        assert_eq!(result.recv_timeout(Duration::from_secs(5)).unwrap(), Ok(()));
        std::fs::write(directory.0.join("release"), []).unwrap();
        wait_for(&directory.0.join("alive"));
        worker.join().unwrap();
    }

    fn execute(command: Command) -> Result<(), String> {
        let mut result = None;
        Runner::default().execute(command, TIME_LIMIT, |completed| result = Some(completed));
        result.unwrap()
    }

    #[test]
    fn success_is_silent_and_stdin_is_closed() {
        assert_eq!(
            execute(fixture("if read value; then exit 99; fi; echo out; echo err >&2")),
            Ok(())
        );
    }

    #[test]
    fn failure_reports_the_status_and_only_the_last_stderr_lines() {
        let error = execute(fixture(
            "echo stdout; printf '%s\\n' old first second third fourth >&2; exit 7",
        ))
        .unwrap_err();
        assert_eq!(error, "exit status: 7\nfirst\nsecond\nthird\nfourth");
    }

    #[test]
    fn bounds_stderr_even_without_line_breaks() {
        let error = execute(fixture("printf '%02000d' 0 >&2; exit 7")).unwrap_err();
        assert_eq!(error, format!("exit status: 7\n{}", "0".repeat(STDERR_BYTES)));
    }

    #[test]
    fn times_out_a_foreground_command_even_after_stderr_closes() {
        let command = fixture("exec 2>&-; exec sleep 15");
        let mut result = None;
        Runner::default().execute(command, Duration::from_millis(250), |completed| {
            result = Some(completed)
        });
        assert_eq!(result.unwrap(), Err("Command timed out.".into()));
    }

    #[test]
    fn quitting_kills_foreground_groups_and_prevents_pending_starts() {
        let directory = TestDirectory::new("quit");
        let mut command = fixture("sleep 15 & touch ready; wait");
        command.current_dir(&directory.0);
        let runner = Arc::new(Runner::default());
        let worker = runner.clone();
        let (ended, completion) = mpsc::channel();
        thread::spawn(move || {
            worker.execute(command, TIME_LIMIT, |result| assert_eq!(result, Ok(())));
            ended.send(()).unwrap();
        });
        wait_for(&directory.0.join("ready"));
        runner.stop();
        completion.recv_timeout(Duration::from_secs(5)).unwrap();

        let mut command = fixture("touch unexpected");
        command.current_dir(&directory.0);
        runner.execute(command, TIME_LIMIT, |result| assert_eq!(result, Ok(())));
        assert!(!directory.0.join("unexpected").exists());
    }
}
