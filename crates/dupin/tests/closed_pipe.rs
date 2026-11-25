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
