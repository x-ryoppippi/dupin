//! Starting a child process that holds only the descriptors it is given.
//!
//! On macOS, std creates sockets, socket pairs, pipes and accepted connections first and marks
//! them close-on-exec afterwards, since macOS has no `SOCK_CLOEXEC`, `pipe2` or `accept4`. A
//! child that [`std::process::Command`] starts on another thread in that gap keeps the
//! descriptor for good. [`Command`] starts every child with `posix_spawn` and
//! `POSIX_SPAWN_CLOEXEC_DEFAULT` instead, so the child gets its standard streams and the
//! descriptors [`Command::fd`] names, and nothing else, whatever the rest of the process opens
//! meanwhile. It never forks, so no atfork handler runs in a child of a multithreaded process.
//!
//! Its API follows [`std::process::Command`]'s, with the same defaults: [`Command::spawn`] and
//! [`Command::status`] inherit the standard streams, and [`Command::output`] gives the child no
//! input and captures its output.

use std::collections::BTreeMap;
use std::ffi::{CString, OsStr, OsString, c_char, c_int};
use std::fs::File;
use std::io::{self, Read};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, ChildStdout, ExitStatus, Output};
use std::ptr;

/// Where `execvp` looks for a program when the child's environment has no `PATH`.
const DEFAULT_PATH: &str = "/usr/bin:/bin";

/// A process to start, built like [`std::process::Command`].
#[derive(Debug)]
pub struct Command {
    program: OsString,
    args: Vec<OsString>,
    clear_env: bool,
    env: BTreeMap<OsString, Option<OsString>>,
    current_dir: Option<PathBuf>,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
    descriptors: Vec<(RawFd, OwnedFd)>,
    process_group: Option<libc::pid_t>,
}

/// What one of a child's standard streams is connected to.
#[derive(Debug)]
pub struct Stdio(Source);

#[derive(Debug)]
enum Source {
    Null,
    Piped,
    Fd(OwnedFd),
}

impl Stdio {
    /// `/dev/null`.
    pub fn null() -> Self {
        Self(Source::Null)
    }

    /// A new pipe, whose other end the [`Child`] holds.
    pub fn piped() -> Self {
        Self(Source::Piped)
    }
}

impl From<OwnedFd> for Stdio {
    fn from(fd: OwnedFd) -> Self {
        Self(Source::Fd(fd))
    }
}

impl From<File> for Stdio {
    fn from(file: File) -> Self {
        Self(Source::Fd(file.into()))
    }
}

impl Command {
    /// `program`, found on the child's `PATH` unless it names a path.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            clear_env: false,
            env: BTreeMap::new(),
            current_dir: None,
            stdin: None,
            stdout: None,
            stderr: None,
            descriptors: Vec::new(),
            process_group: None,
        }
    }

    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        self
    }

    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.env
            .insert(key.as_ref().to_owned(), Some(value.as_ref().to_owned()));
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        for (key, value) in vars {
            self.env(key, value);
        }
        self
    }

    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.env.insert(key.as_ref().to_owned(), None);
        self
    }

    /// Starts the child with no environment but what [`Command::env`] adds afterwards.
    pub fn env_clear(&mut self) -> &mut Self {
        self.clear_env = true;
        self.env.clear();
        self
    }

    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.current_dir = Some(dir.as_ref().to_owned());
        self
    }

    pub fn stdin(&mut self, stdin: impl Into<Stdio>) -> &mut Self {
        self.stdin = Some(stdin.into());
        self
    }

    pub fn stdout(&mut self, stdout: impl Into<Stdio>) -> &mut Self {
        self.stdout = Some(stdout.into());
        self
    }

    pub fn stderr(&mut self, stderr: impl Into<Stdio>) -> &mut Self {
        self.stderr = Some(stderr.into());
        self
    }

    /// Gives the child `source` as descriptor `target`, which must be above the standard
    /// streams.
    pub fn fd(&mut self, target: RawFd, source: impl Into<OwnedFd>) -> &mut Self {
        assert!(
            target > libc::STDERR_FILENO,
            "descriptor {target} is a standard stream"
        );
        self.descriptors.retain(|&(taken, _)| taken != target);
        self.descriptors.push((target, source.into()));
        self
    }

    /// Puts the child in process group `group`, or, with 0, in a new group it leads.
    pub fn process_group(&mut self, group: libc::pid_t) -> &mut Self {
        self.process_group = Some(group);
        self
    }

    pub fn get_program(&self) -> &OsStr {
        &self.program
    }

    pub fn get_args(&self) -> impl Iterator<Item = &OsStr> {
        self.args.iter().map(OsString::as_os_str)
    }

    /// The variables this command sets, and, as `None`, those it removes.
    pub fn get_envs(&self) -> impl Iterator<Item = (&OsStr, Option<&OsStr>)> {
        self.env
            .iter()
            .map(|(key, value)| (key.as_os_str(), value.as_deref()))
    }

    /// Starts the child, by default with the parent's standard streams.
    pub fn spawn(&mut self) -> io::Result<Child> {
        self.start(Defaults::Inherit)
    }

    /// Runs the child to completion, by default with the parent's standard streams.
    pub fn status(&mut self) -> io::Result<ExitStatus> {
        self.start(Defaults::Inherit)?.wait()
    }

    /// Runs the child to completion, by default with no input and its output captured.
    pub fn output(&mut self) -> io::Result<Output> {
        self.start(Defaults::Capture)?.wait_with_output()
    }

    fn start(&self, defaults: Defaults) -> io::Result<Child> {
        let mut actions = FileActions::new()?;
        let mut mapped: Vec<(RawFd, RawFd)> = Vec::new();
        let mut child_ends = Vec::new();
        let mut parent_ends: [Option<OwnedFd>; 3] = [None, None, None];
        for (target, stdio) in [
            (libc::STDIN_FILENO, &self.stdin),
            (libc::STDOUT_FILENO, &self.stdout),
            (libc::STDERR_FILENO, &self.stderr),
        ] {
            let plan = match stdio {
                Some(Stdio(Source::Null)) => Plan::Null,
                Some(Stdio(Source::Piped)) => Plan::Piped,
                Some(Stdio(Source::Fd(fd))) => Plan::Fd(fd.as_raw_fd()),
                None => defaults.of(target),
            };
            match plan {
                Plan::Inherit => actions.inherit(target)?,
                Plan::Null => {
                    let access = if target == libc::STDIN_FILENO {
                        libc::O_RDONLY
                    } else {
                        libc::O_WRONLY
                    };
                    actions.open(target, c"/dev/null", access)?;
                }
                Plan::Piped => {
                    let (reader, writer) = io::pipe()?;
                    let (ours, theirs) = if target == libc::STDIN_FILENO {
                        (OwnedFd::from(writer), OwnedFd::from(reader))
                    } else {
                        (OwnedFd::from(reader), OwnedFd::from(writer))
                    };
                    mapped.push((theirs.as_raw_fd(), target));
                    child_ends.push(theirs);
                    parent_ends[target as usize] = Some(ours);
                }
                Plan::Fd(fd) => mapped.push((fd, target)),
            }
        }
        mapped.extend(
            self.descriptors
                .iter()
                .map(|(target, source)| (source.as_raw_fd(), *target)),
        );
        // A source may sit on another mapping's target, which an earlier dup2 in the child would
        // overwrite, so every source moves above all targets first.
        let above_targets = mapped
            .iter()
            .map(|&(_, target)| target)
            .max()
            .unwrap_or(libc::STDERR_FILENO)
            .max(libc::STDERR_FILENO)
            + 1;
        let lifted = mapped
            .iter()
            .map(|&(source, _)| duplicate_above(source, above_targets))
            .collect::<io::Result<Vec<_>>>()?;
        for (fd, &(_, target)) in lifted.iter().zip(&mapped) {
            actions.dup2(fd.as_raw_fd(), target)?;
        }
        if let Some(dir) = &self.current_dir {
            actions.chdir(&CString::new(dir.as_os_str().as_bytes())?)?;
        }
        let attributes = Attributes::new(self.process_group)?;

        let environment = self.environment();
        let path = self.resolve(&environment)?;
        let argv_strings = std::iter::once(&self.program)
            .chain(&self.args)
            .map(|arg| CString::new(arg.as_bytes()))
            .collect::<Result<Vec<_>, _>>()?;
        let argv: Vec<*mut c_char> = argv_strings
            .iter()
            .map(|arg| arg.as_ptr().cast_mut())
            .chain(std::iter::once(ptr::null_mut()))
            .collect();
        let envp_strings = environment
            .iter()
            .map(|(key, value)| {
                let mut pair = Vec::with_capacity(key.len() + value.len() + 1);
                pair.extend_from_slice(key.as_bytes());
                pair.push(b'=');
                pair.extend_from_slice(value.as_bytes());
                CString::new(pair)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let envp: Vec<*mut c_char> = envp_strings
            .iter()
            .map(|pair| pair.as_ptr().cast_mut())
            .chain(std::iter::once(ptr::null_mut()))
            .collect();

        let mut pid = 0;
        // SAFETY: every pointer is valid for the call, and argv and envp are null-terminated.
        let code = unsafe {
            libc::posix_spawn(
                &raw mut pid,
                path.as_ptr(),
                &raw const actions.0,
                &raw const attributes.0,
                argv.as_ptr(),
                envp.as_ptr(),
            )
        };
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code));
        }
        let [stdin, stdout, stderr] = parent_ends;
        Ok(Child {
            pid,
            status: None,
            stdin: stdin.map(ChildStdin::from),
            stdout: stdout.map(ChildStdout::from),
            stderr: stderr.map(ChildStderr::from),
        })
    }

    /// The child's environment: the parent's with this command's changes, or only what was set
    /// after [`Command::env_clear`].
    fn environment(&self) -> Vec<(OsString, OsString)> {
        let mut environment: BTreeMap<OsString, OsString> = if self.clear_env {
            BTreeMap::new()
        } else {
            std::env::vars_os().collect()
        };
        for (key, value) in &self.env {
            match value {
                Some(value) => environment.insert(key.clone(), value.clone()),
                None => environment.remove(key),
            };
        }
        environment.into_iter().collect()
    }

    /// The program to execute: itself when it names a path, or else the first executable of
    /// that name on the child's `PATH`, as `execvp` searches it.
    fn resolve(&self, environment: &[(OsString, OsString)]) -> io::Result<CString> {
        if self.program.as_bytes().contains(&b'/') {
            return Ok(CString::new(self.program.as_bytes())?);
        }
        let search = environment
            .iter()
            .find(|(key, _)| key == "PATH")
            .map_or(OsStr::new(DEFAULT_PATH), |(_, value)| value.as_os_str());
        for dir in std::env::split_paths(search) {
            let dir = if dir.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                dir
            };
            let candidate = CString::new(dir.join(&self.program).into_os_string().as_bytes())?;
            // SAFETY: `candidate` is a valid C string for the call.
            let executable = unsafe { libc::access(candidate.as_ptr(), libc::X_OK) } == 0;
            if executable && Path::new(OsStr::from_bytes(candidate.as_bytes())).is_file() {
                return Ok(candidate);
            }
        }
        Err(io::Error::from_raw_os_error(libc::ENOENT))
    }
}

#[derive(Clone, Copy)]
enum Defaults {
    Inherit,
    Capture,
}

impl Defaults {
    fn of(self, target: RawFd) -> Plan {
        match (self, target) {
            (Self::Inherit, _) => Plan::Inherit,
            (Self::Capture, libc::STDIN_FILENO) => Plan::Null,
            (Self::Capture, _) => Plan::Piped,
        }
    }
}

/// How one standard stream of the child is set up.
#[derive(Clone, Copy)]
enum Plan {
    Inherit,
    Null,
    Piped,
    Fd(RawFd),
}

/// A started child, like [`std::process::Child`]: dropping it neither kills nor reaps it.
#[derive(Debug)]
pub struct Child {
    pid: libc::pid_t,
    status: Option<ExitStatus>,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
}

impl Child {
    pub fn id(&self) -> u32 {
        self.pid as u32
    }

    /// Sends SIGKILL, unless the child was already reaped.
    pub fn kill(&mut self) -> io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        // SAFETY: the child is not yet reaped, so its pid can't belong to another process.
        if unsafe { libc::kill(self.pid, libc::SIGKILL) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Closes the child's input, then waits for it to exit.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        drop(self.stdin.take());
        if let Some(status) = self.status {
            return Ok(status);
        }
        let mut raw = 0;
        // SAFETY: waitpid writes only to `raw`.
        while unsafe { libc::waitpid(self.pid, &raw mut raw, 0) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
        let status = ExitStatus::from_raw(raw);
        self.status = Some(status);
        Ok(status)
    }

    /// The child's exit status if it has exited, without waiting.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        let mut raw = 0;
        // SAFETY: waitpid writes only to `raw`.
        match unsafe { libc::waitpid(self.pid, &raw mut raw, libc::WNOHANG) } {
            0 => Ok(None),
            pid if pid < 0 => Err(io::Error::last_os_error()),
            _ => {
                let status = ExitStatus::from_raw(raw);
                self.status = Some(status);
                Ok(Some(status))
            }
        }
    }

    /// Closes the child's input, reads its piped output to the end and waits for it to exit.
    pub fn wait_with_output(mut self) -> io::Result<Output> {
        drop(self.stdin.take());
        let (stdout, stderr) = read_both(self.stdout.take(), self.stderr.take())?;
        let status = self.wait()?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
}

/// Reads both streams to the end at once, so a child filling one pipe never stalls on it while
/// the other is read.
fn read_both(stdout: Option<ChildStdout>, stderr: Option<ChildStderr>) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    let mut err = Vec::new();
    match (stdout, stderr) {
        (None, None) => {}
        (Some(mut stdout), None) => {
            stdout.read_to_end(&mut out)?;
        }
        (None, Some(mut stderr)) => {
            stderr.read_to_end(&mut err)?;
        }
        (Some(stdout), Some(stderr)) => {
            let mut streams = [
                (Some(File::from(OwnedFd::from(stdout))), &mut out),
                (Some(File::from(OwnedFd::from(stderr))), &mut err),
            ];
            for (file, _) in &streams {
                set_nonblocking(file.as_ref().expect("open"))?;
            }
            loop {
                let mut fds: Vec<libc::pollfd> = streams
                    .iter()
                    .filter_map(|(file, _)| file.as_ref())
                    .map(|file| libc::pollfd {
                        fd: file.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    })
                    .collect();
                if fds.is_empty() {
                    break;
                }
                // SAFETY: `fds` is valid for its length.
                if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) } < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                for (file, bytes) in &mut streams {
                    let Some(open) = file else { continue };
                    match open.read_to_end(bytes) {
                        Ok(_) => *file = None,
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                            ) => {}
                        Err(error) => return Err(error),
                    }
                }
            }
        }
    }
    Ok((out, err))
}

fn set_nonblocking(file: &File) -> io::Result<()> {
    // SAFETY: fcntl only reads and sets the status flags of a descriptor `file` keeps open.
    unsafe {
        let flags = libc::fcntl(file.as_raw_fd(), libc::F_GETFL);
        if flags < 0 || libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// A close-on-exec copy of `fd` numbered at least `floor`.
fn duplicate_above(fd: RawFd, floor: RawFd) -> io::Result<OwnedFd> {
    // SAFETY: fcntl only duplicates a descriptor the caller keeps open.
    let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, floor) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl just returned this descriptor, and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(copy) })
}

unsafe extern "C" {
    /// Keeps `fd` open in the child despite `POSIX_SPAWN_CLOEXEC_DEFAULT`.
    fn posix_spawn_file_actions_addinherit_np(
        actions: *mut libc::posix_spawn_file_actions_t,
        fd: c_int,
    ) -> c_int;
}

struct FileActions(libc::posix_spawn_file_actions_t);

impl FileActions {
    fn new() -> io::Result<Self> {
        let mut actions = MaybeUninit::uninit();
        // SAFETY: init fills `actions`, which is used only once it succeeded.
        check(unsafe { libc::posix_spawn_file_actions_init(actions.as_mut_ptr()) })?;
        Ok(Self(unsafe { actions.assume_init() }))
    }

    fn inherit(&mut self, fd: RawFd) -> io::Result<()> {
        // SAFETY: `self.0` was initialized by `new`.
        check(unsafe { posix_spawn_file_actions_addinherit_np(&raw mut self.0, fd) })
    }

    fn open(&mut self, fd: RawFd, path: &std::ffi::CStr, flags: c_int) -> io::Result<()> {
        // SAFETY: `self.0` was initialized by `new`, and the action copies `path`.
        check(unsafe { libc::posix_spawn_file_actions_addopen(&raw mut self.0, fd, path.as_ptr(), flags, 0) })
    }

    fn dup2(&mut self, source: RawFd, target: RawFd) -> io::Result<()> {
        // SAFETY: `self.0` was initialized by `new`.
        check(unsafe { libc::posix_spawn_file_actions_adddup2(&raw mut self.0, source, target) })
    }

    fn chdir(&mut self, dir: &std::ffi::CStr) -> io::Result<()> {
        // SAFETY: `self.0` was initialized by `new`, and the action copies `dir`.
        check(unsafe { libc::posix_spawn_file_actions_addchdir_np(&raw mut self.0, dir.as_ptr()) })
    }
}

impl Drop for FileActions {
    fn drop(&mut self) {
        // SAFETY: `self.0` was initialized by `new` and is destroyed once.
        unsafe { libc::posix_spawn_file_actions_destroy(&raw mut self.0) };
    }
}

struct Attributes(libc::posix_spawnattr_t);

impl Attributes {
    /// No descriptor beyond those the file actions name, an empty signal mask and the default
    /// SIGPIPE disposition instead of the parent's, as std gives a child, and `group` if any.
    fn new(group: Option<libc::pid_t>) -> io::Result<Self> {
        let mut attributes = MaybeUninit::uninit();
        // SAFETY: init fills `attributes`, which is used only once it succeeded.
        check(unsafe { libc::posix_spawnattr_init(attributes.as_mut_ptr()) })?;
        let mut this = Self(unsafe { attributes.assume_init() });
        let mut flags =
            libc::POSIX_SPAWN_CLOEXEC_DEFAULT | libc::POSIX_SPAWN_SETSIGDEF | libc::POSIX_SPAWN_SETSIGMASK;
        if let Some(group) = group {
            flags |= libc::POSIX_SPAWN_SETPGROUP;
            // SAFETY: `this.0` was initialized by init.
            check(unsafe { libc::posix_spawnattr_setpgroup(&raw mut this.0, group) })?;
        }
        let mut empty = MaybeUninit::uninit();
        let mut pipe = MaybeUninit::uninit();
        // SAFETY: each set is initialized by sigemptyset before it is read, and `this.0` by init.
        unsafe {
            libc::sigemptyset(empty.as_mut_ptr());
            libc::sigemptyset(pipe.as_mut_ptr());
            libc::sigaddset(pipe.as_mut_ptr(), libc::SIGPIPE);
            check(libc::posix_spawnattr_setsigmask(&raw mut this.0, empty.as_ptr()))?;
            check(libc::posix_spawnattr_setsigdefault(
                &raw mut this.0,
                pipe.as_ptr(),
            ))?;
            check(libc::posix_spawnattr_setflags(
                &raw mut this.0,
                flags as libc::c_short,
            ))?;
        }
        Ok(this)
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: `self.0` was initialized by `new` and is destroyed once.
        unsafe { libc::posix_spawnattr_destroy(&raw mut self.0) };
    }
}

fn check(code: c_int) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code))
    }
}

#[cfg(test)]
mod tests;
