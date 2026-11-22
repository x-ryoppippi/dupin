use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, PartialEq, Eq)]
enum Target {
    Bundle(PathBuf),
    Executable(PathBuf),
}

fn current_target(executable: PathBuf, bundle: PathBuf) -> Target {
    if executable.starts_with(bundle.join("Contents/MacOS")) {
        Target::Bundle(bundle)
    } else {
        Target::Executable(executable)
    }
}

pub fn current() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| format!("couldn't locate Dupin: {e}"))?;
    restart(current_target(executable, dupin_mac::bundle::path()))
}

pub fn bundle(path: &Path) -> Result<(), String> {
    restart(Target::Bundle(path.to_owned()))
}

fn waiter(target: &Target, pid: u32) -> Command {
    // Launch after exit so the replacement can acquire the global shortcut.
    let (script, path) = match target {
        Target::Bundle(path) => (
            "while /bin/kill -0 \"$1\" 2>/dev/null; do /bin/sleep 0.1; done; exec /usr/bin/open \"$2\"",
            path,
        ),
        Target::Executable(path) => (
            "while /bin/kill -0 \"$1\" 2>/dev/null; do /bin/sleep 0.1; done; exec \"$2\"",
            path,
        ),
    };
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", script, "sh"])
        .arg(pid.to_string())
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn restart(target: Target) -> Result<(), String> {
    waiter(&target, std::process::id())
        .spawn()
        .map_err(|e| format!("couldn't relaunch Dupin: {e}"))?;
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packaged_app_reopens_its_bundle_but_a_bare_build_runs_its_binary() {
        let bundle = PathBuf::from("/Applications/My Dupin.app");
        assert_eq!(
            current_target(bundle.join("Contents/MacOS/dupin"), bundle.clone()),
            Target::Bundle(bundle)
        );
        let executable = PathBuf::from("/work/target/debug/dupin");
        assert_eq!(
            current_target(executable.clone(), PathBuf::from("/work/target/debug")),
            Target::Executable(executable)
        );
    }

    #[test]
    fn paths_are_positional_arguments_instead_of_shell_source() {
        let path = PathBuf::from("/Applications/Dupin's $(touch marker).app");
        for target in [Target::Bundle(path.clone()), Target::Executable(path.clone())] {
            let command = waiter(&target, 1234);
            assert_eq!(command.get_program(), "/bin/sh");
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(args.len(), 5);
            assert_eq!(args[3], "1234");
            assert_eq!(args[4], path.as_os_str());
            assert!(!args[1].to_string_lossy().contains("touch marker"));
            assert_eq!(
                args[1].to_string_lossy().contains("/usr/bin/open"),
                matches!(target, Target::Bundle(_))
            );
        }
    }

    #[test]
    fn the_replacement_waits_for_the_original_process_to_exit() {
        let mut original = Command::new("/bin/sleep").arg("10").spawn().unwrap();
        let mut replacement = waiter(&Target::Executable(PathBuf::from("/usr/bin/true")), original.id())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));
        let early = replacement.try_wait().unwrap();
        original.kill().unwrap();
        original.wait().unwrap();
        assert!(early.is_none(), "replacement ran while the original was alive");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Some(status) = replacement.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if std::time::Instant::now() >= deadline {
                replacement.kill().unwrap();
                replacement.wait().unwrap();
                panic!("replacement did not start after the original exited");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
