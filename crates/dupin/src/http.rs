use dupin_spawn::Command;
use std::time::Duration;

pub fn curl(max: Duration, options: &[&str]) -> Command {
    let mut command = Command::new("/usr/bin/curl");
    command
        .args(options)
        .args(["--fail", "--silent", "--show-error"])
        .args(["--connect-timeout", "30", "--max-time"])
        .arg(max.as_secs().to_string());
    command
}
