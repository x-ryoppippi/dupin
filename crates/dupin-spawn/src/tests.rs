use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::*;

/// Threads that open and close sockets, socket pairs and pipes as fast as they can until
/// dropped, each one, between its creation and std's `fcntl`, a descriptor a child spawned
/// meanwhile would keep.
struct Churn {
    stop: Arc<AtomicBool>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl Churn {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let threads = (0..4)
            .map(|_| {
                let stop = stop.clone();
                thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        drop(UnixStream::pair());
                        drop(io::pipe());
                        drop(std::net::UdpSocket::bind("127.0.0.1:0"));
                    }
                })
            })
            .collect();
        Self { stop, threads }
    }
}

impl Drop for Churn {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            thread.join().unwrap();
        }
    }
}

/// The descriptors `ls -l /dev/fd/` printed, by number, with each one's type: the first letter
/// of its mode. `ls` adds the directories it opens to read `/dev/fd` itself.
fn table(listing: &[u8]) -> Vec<(RawFd, char)> {
    String::from_utf8_lossy(listing)
        .lines()
        .filter_map(|line| {
            let kind = line.chars().next()?;
            let fd = line.split_whitespace().last()?.parse().ok()?;
            Some((fd, kind))
        })
        .collect()
}

/// What `ls` held beyond the standard streams, `mapped` and the directories it opened itself.
fn unexpected(table: &[(RawFd, char)], mapped: &[RawFd]) -> Vec<(RawFd, char)> {
    table
        .iter()
        .copied()
        .filter(|&(fd, kind)| fd > libc::STDERR_FILENO && !mapped.contains(&fd) && kind != 'd')
        .collect()
}

const ROUNDS: usize = 300;

#[test]
fn a_child_holds_only_the_descriptors_it_is_given_while_others_are_opened() {
    let churn = Churn::start();
    for _ in 0..ROUNDS {
        let (reader, _writer) = io::pipe().unwrap();
        let output = Command::new("/bin/ls")
            .args(["-l", "/dev/fd/"])
            .fd(3, reader)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let table = table(&output.stdout);
        assert_eq!(
            table.iter().find(|&&(fd, _)| fd == 3),
            Some(&(3, 'p')),
            "{table:?}"
        );
        let leaked = unexpected(&table, &[3]);
        assert!(leaked.is_empty(), "the child inherited {leaked:?}");
    }
    drop(churn);
}

/// The same churn leaks into a child std starts, which is what the test above guards against.
#[test]
#[expect(clippy::disallowed_types, reason = "shows the leak std's children have")]
fn a_std_child_inherits_descriptors_opened_meanwhile() {
    let churn = Churn::start();
    let leaked = (0..ROUNDS * 10).any(|_| {
        let output = std::process::Command::new("/bin/ls")
            .args(["-l", "/dev/fd/"])
            .output()
            .unwrap();
        !unexpected(&table(&output.stdout), &[]).is_empty()
    });
    drop(churn);
    assert!(leaked, "no descriptor leaked into std's children");
}

#[test]
fn output_captures_both_streams_without_stalling_on_either() {
    let output = Command::new("/bin/sh")
        .args([
            "-c",
            "head -c 200000 /dev/zero; head -c 300000 /dev/zero >&2; exit 3",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert_eq!((output.stdout.len(), output.stderr.len()), (200_000, 300_000));
}

#[test]
fn input_reaches_a_piped_stdin_and_closes_on_wait() {
    let mut child = Command::new("/bin/cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"hello").unwrap();
    let mut echoed = String::new();
    child.stdout.take().unwrap().read_to_string(&mut echoed).unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(echoed, "hello");
}

#[test]
fn the_environment_and_directory_are_the_commands() {
    let dir = std::env::temp_dir().canonicalize().unwrap();
    let output = Command::new("/bin/sh")
        .args(["-c", "pwd; env | sort"])
        .env_clear()
        .env("ONLY", "this")
        .current_dir(&dir)
        .output()
        .unwrap();
    let printed = String::from_utf8(output.stdout).unwrap();
    let mut lines = printed.lines();
    assert_eq!(lines.next().map(Path::new), Some(dir.as_path()));
    // The shell sets these itself.
    let variables: Vec<_> = lines
        .filter(|line| !["PWD=", "SHLVL=", "_="].iter().any(|own| line.starts_with(own)))
        .collect();
    assert_eq!(variables, ["ONLY=this"]);

    let inherited = Command::new("/bin/sh")
        .args(["-c", "printf %s \"$PATH|${HOME+set}\""])
        .env_remove("HOME")
        .output()
        .unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    assert_eq!(String::from_utf8(inherited.stdout).unwrap(), format!("{path}|"));
}

#[test]
fn a_bare_name_is_found_on_the_path_the_child_gets() {
    let output = Command::new("true").env_clear().output().unwrap();
    assert!(output.status.success());
    let error = Command::new("true")
        .env("PATH", "/var/empty")
        .output()
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ENOENT));
    assert_eq!(
        Command::new("/nonexistent/program")
            .spawn()
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ENOENT)
    );
}

#[test]
fn a_new_process_group_is_led_by_the_child() {
    let mut child = Command::new("/bin/sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .unwrap();
    let pid = child.id() as libc::pid_t;
    // SAFETY: getpgid only reads the group of a child that isn't reaped yet.
    assert_eq!(unsafe { libc::getpgid(pid) }, pid);
    // SAFETY: the group was just created and is led by the unreaped child.
    assert_eq!(unsafe { libc::killpg(pid, libc::SIGKILL) }, 0);
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
}

#[test]
fn try_wait_and_kill_follow_std() {
    let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
    assert_eq!(child.try_wait().unwrap(), None);
    child.kill().unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert_eq!(child.wait().unwrap(), status);
    child.kill().unwrap();
}

#[test]
fn the_child_starts_with_no_blocked_signals_and_the_default_sigpipe() {
    // SAFETY: blocks SIGUSR1 on this test's own thread only, which exits with the test.
    unsafe {
        let mut set = MaybeUninit::uninit();
        libc::sigemptyset(set.as_mut_ptr());
        libc::sigaddset(set.as_mut_ptr(), libc::SIGUSR1);
        libc::pthread_sigmask(libc::SIG_BLOCK, set.as_ptr(), ptr::null_mut());
    }
    let output = Command::new("/bin/sh")
        .args(["-c", "kill -USR1 $$; sleep 1"])
        .output()
        .unwrap();
    assert_eq!(output.status.signal(), Some(libc::SIGUSR1));
    // The test harness ignores SIGPIPE, and a shell can't take back a disposition it inherits
    // as ignored.
    let output = Command::new("/bin/sh")
        .args(["-c", "kill -PIPE $$; sleep 1"])
        .output()
        .unwrap();
    assert_eq!(output.status.signal(), Some(libc::SIGPIPE));
}
