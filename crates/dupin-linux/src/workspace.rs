//! Running actions through the desktop: a desktop entry launches its `Exec` command, and
//! everything else goes to `xdg-open`, which hands it to the desktop's default handler.

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::desktop_entry::{self, Locale};

#[derive(Debug)]
pub enum Action {
    Open(PathBuf),
    OpenApp {
        bundle_id: String,
        hide_if_frontmost: bool,
    },
    Reveal(PathBuf),
    OpenUrl(String),
    OpenWith {
        path: PathBuf,
        application: PathBuf,
    },
    Trash(PathBuf),
}

/// Carry out `action`, or say why the system refused it (a vanished file, say). Blocks until
/// `xdg-open` returns or `HANDOFF` passes, so callers run it off the main thread. Linux results
/// offer no "Open with…" or "Move to Trash", so those actions are refused.
pub fn run(action: &Action) -> Result<(), String> {
    match action {
        Action::Open(path) => open(path)
            .then_some(())
            .ok_or_else(|| format!("Couldn't open “{}”.", path.display())),
        Action::OpenUrl(url) => xdg_open(url.as_ref())
            .then_some(())
            .ok_or_else(|| "Couldn't open this link.".into()),
        Action::Reveal(path) => {
            if !path.exists() {
                return Err(format!("“{}” no longer exists.", path.display()));
            }
            reveal(path)
                .then_some(())
                .ok_or_else(|| format!("Couldn't show “{}”.", path.display()))
        }
        Action::OpenApp { .. } => Err("Opening an app by its bundle ID isn't available on Linux.".into()),
        Action::OpenWith { .. } => Err("Opening with another application isn't available on Linux.".into()),
        Action::Trash(_) => Err("Moving to the Trash isn't available on Linux.".into()),
    }
}

/// An application that can open a file, as "Open with…" lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub path: PathBuf,
    pub name: String,
    pub default: bool,
}

/// The applications that can open `path`. Linux results offer no "Open with…", so this is
/// refused.
pub fn applications_for(_path: &Path) -> Result<Vec<Application>, String> {
    Err("Choosing an application isn't available on Linux.".into())
}

/// Open with the default handler: launches a desktop entry's app, opens a document or folder.
pub fn open(path: &Path) -> bool {
    if path.extension().is_some_and(|e| e == "desktop") {
        launch(path)
    } else {
        xdg_open(path.as_os_str())
    }
}

/// Show the folder that holds `path`. The file manager opens the folder; selecting the item in
/// it needs the FileManager1 D-Bus interface, which not every desktop has.
fn reveal(path: &Path) -> bool {
    let folder = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    xdg_open(folder.as_os_str())
}

/// How long `xdg-open` has to report a failure. Some desktops' `xdg-open` runs the handler in
/// the foreground and returns only when it exits; one still running by then counts as opened.
const HANDOFF: Duration = Duration::from_secs(2);

fn xdg_open(target: &std::ffi::OsStr) -> bool {
    let Ok(mut child) = Command::new("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + HANDOFF;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                std::thread::spawn(move || child.wait());
                return true;
            }
            Err(_) => return false,
        }
    }
}

/// Start a desktop entry's app in a session of its own, so it outlives Dupin.
fn launch(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Some(entry) = desktop_entry::parse(&text, &Locale::from_env(), &desktop_entry::current_desktops())
    else {
        return false;
    };
    let Some(mut args) = entry
        .exec
        .as_deref()
        .and_then(|exec| desktop_entry::command(exec, &entry, path))
    else {
        return false;
    };
    if entry.terminal {
        let Some(mut terminal) = terminal() else {
            return false;
        };
        terminal.append(&mut args);
        args = terminal;
    }
    let mut command = Command::new(&args[0]);
    command
        .args(&args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = &entry.working_dir {
        command.current_dir(dir);
    }
    // SAFETY: setsid is async-signal-safe, and nothing else runs between fork and exec.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    match command.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
            true
        }
        Err(_) => false,
    }
}

/// The command that runs a program in a terminal: `$TERMINAL -e`, else `xdg-terminal-exec`,
/// else XFCE's `exo-open --launch TerminalEmulator`. None when there is none of them.
fn terminal() -> Option<Vec<OsString>> {
    terminal_from(
        std::env::var_os("TERMINAL").filter(|t| !t.is_empty()),
        desktop_entry::find_executable,
    )
}

fn terminal_from(
    variable: Option<OsString>,
    find: impl Fn(&str) -> Option<PathBuf>,
) -> Option<Vec<OsString>> {
    if let Some(terminal) = variable {
        return Some(vec![terminal, "-e".into()]);
    }
    if let Some(path) = find("xdg-terminal-exec") {
        return Some(vec![path.into_os_string()]);
    }
    find("exo-open").map(|path| {
        vec![
            path.into_os_string(),
            "--launch".into(),
            "TerminalEmulator".into(),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_terminal_variable_then_the_launchers() {
        let none = |_: &str| None;
        let only = |name: &'static str| {
            move |program: &str| (program == name).then(|| PathBuf::from(format!("/bin/{name}")))
        };
        assert_eq!(
            terminal_from(Some("kitty".into()), only("exo-open")),
            Some(vec!["kitty".into(), "-e".into()])
        );
        assert_eq!(
            terminal_from(None, only("xdg-terminal-exec")),
            Some(vec!["/bin/xdg-terminal-exec".into()])
        );
        assert_eq!(
            terminal_from(None, only("exo-open")),
            Some(vec![
                "/bin/exo-open".into(),
                "--launch".into(),
                "TerminalEmulator".into()
            ])
        );
        assert_eq!(terminal_from(None, none), None);
    }
}
