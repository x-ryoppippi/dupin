use std::{
    collections::VecDeque,
    env,
    io::{self, Read},
    os::{fd::OwnedFd, unix::net::UnixStream},
    process::{Child, Command, ExitStatus, Stdio},
    time::Duration,
};

const STDERR_BYTES: usize = 512;
const STDERR_LINES: usize = 4;

pub fn run(line: &str) -> Result<(), String> {
    let home = env::home_dir().ok_or("Couldn't determine the home directory.")?;
    let shell = env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    let mut command = Command::new(shell);
    command.args(["-lc", line]).current_dir(home);
    execute(command)
}

fn execute(mut command: Command) -> Result<(), String> {
    let (stderr, writer) = UnixStream::pair().map_err(|error| error.to_string())?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(OwnedFd::from(writer))
        .spawn()
        .map_err(|error| error.to_string())?;
    drop(command);

    let mut tail = VecDeque::with_capacity(STDERR_BYTES);
    let status = match collect(&mut child, stderr, &mut tail) {
        Ok(status) => status,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.to_string());
        }
    };
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

fn collect(child: &mut Child, mut stderr: UnixStream, tail: &mut VecDeque<u8>) -> io::Result<ExitStatus> {
    // A background descendant can keep stderr open after the shell exits.
    stderr.set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut bytes = [0; 8192];
    loop {
        match stderr.read(&mut bytes) {
            Ok(0) => return child.wait(),
            Ok(count) => keep_tail(tail, &bytes[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        if let Some(status) = child.try_wait()? {
            if !status.success() && stderr.set_nonblocking(true).is_ok() {
                // Drain buffered output without letting a descendant extend the run.
                let mut pending = stderr.take(256 * 1024);
                while let Ok(count @ 1..) = pending.read(&mut bytes) {
                    keep_tail(tail, &bytes[..count]);
                }
            }
            return Ok(status);
        }
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

    fn fixture(line: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", line]);
        command
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
    fn reaps_the_shell_when_stderr_is_still_open() {
        let (stderr, _writer) = UnixStream::pair().unwrap();
        let mut child = fixture("exit 7").spawn().unwrap();
        assert_eq!(
            collect(&mut child, stderr, &mut VecDeque::new()).unwrap().code(),
            Some(7)
        );
    }
}
