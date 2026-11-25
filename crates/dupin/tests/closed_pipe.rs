//! A reader that closes the pipe early (`dupin ... --json | head`) is not a failure: the CLI
//! exits quietly instead of panicking on EPIPE.
#![cfg(unix)]

use std::fs::File;
use std::os::fd::FromRawFd;
use std::process::{Command, Stdio};

/// A stdout whose read end is already closed, so every write fails with EPIPE.
fn closed_pipe() -> Stdio {
    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    unsafe { libc::close(fds[0]) };
    Stdio::from(unsafe { File::from_raw_fd(fds[1]) })
}

fn exits_quietly(args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_dupin"))
        .args(args)
        .stdout(closed_pipe())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "{args:?} panicked: {stderr}");
    assert!(
        output.status.success(),
        "{args:?} exited {:?}: {stderr}",
        output.status
    );
}

#[test]
fn json_output_survives_a_closed_pipe() {
    exits_quietly(&["--version", "--json"]);
}

#[test]
fn plain_output_survives_a_closed_pipe() {
    exits_quietly(&["--help"]);
    exits_quietly(&["--version"]);
}

#[test]
fn other_write_errors_fail_the_command() {
    use std::os::unix::process::CommandExt;
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_dupin"));
    command
        .args(["--version", "--json"])
        .stdout(Stdio::from(file.reopen().unwrap()))
        .stderr(Stdio::piped());
    // A zero file-size limit makes every write to the file fail with EFBIG, standing in for a
    // full disk behind a redirect; SIGXFSZ is ignored so the write returns the error.
    unsafe {
        command.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: libc::RLIM_INFINITY,
            };
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let output = command.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "panicked: {stderr}");
    assert!(!output.status.success(), "a failed write exited successfully");
    assert!(stderr.contains("couldn't write to stdout"), "{stderr}");
}
