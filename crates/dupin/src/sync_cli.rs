//! `dupin sync`: reads a passphrase without putting it in the command line, and prints what the
//! running launcher's sync service answers.

use std::io::{self, BufRead, IsTerminal, Write};

use serde_json::Value;

use crate::control::SyncAction;

const MAX_PASSPHRASE: usize = 1024;

/// Fills in the passphrase: from stdin with `--passphrase-stdin`, or, to join from a terminal,
/// typed at a prompt that doesn't echo. Left out, `on` generates one and `join` relies on
/// iCloud Keychain.
pub fn with_passphrase(action: SyncAction, from_stdin: bool) -> Result<SyncAction, String> {
    let read = |action: &SyncAction| -> Result<Option<String>, String> {
        if from_stdin {
            return read_line(&mut io::stdin().lock()).map(Some);
        }
        if matches!(action, SyncAction::Join { .. }) && io::stdin().is_terminal() {
            let typed = prompt("Passphrase (Return to use iCloud Keychain): ")?;
            return Ok((!typed.is_empty()).then_some(typed));
        }
        Ok(None)
    };
    Ok(match action {
        SyncAction::On { folder, .. } => SyncAction::On {
            passphrase: read(&SyncAction::On {
                folder: None,
                passphrase: None,
            })?,
            folder: absolute(folder)?,
        },
        SyncAction::Join { folder, .. } => SyncAction::Join {
            passphrase: read(&SyncAction::Join {
                folder: None,
                passphrase: None,
            })?,
            folder: absolute(folder)?,
        },
        other => other,
    })
}

/// The service runs in the launcher, whose working directory isn't this shell's.
fn absolute(folder: Option<String>) -> Result<Option<String>, String> {
    let Some(folder) = folder else {
        return Ok(None);
    };
    if folder.starts_with('/') || folder.starts_with("~/") {
        return Ok(Some(folder));
    }
    let cwd = std::env::current_dir().map_err(|_| "couldn't read the current folder")?;
    Ok(Some(cwd.join(folder).display().to_string()))
}

fn read_line(input: impl BufRead) -> Result<String, String> {
    let mut line = String::new();
    input
        .take(MAX_PASSPHRASE as u64 + 2)
        .read_line(&mut line)
        .map_err(|_| "couldn't read the passphrase")?;
    let trimmed = line.trim_end_matches(['\n', '\r']).to_owned();
    zeroize::Zeroize::zeroize(&mut line);
    if trimmed.len() > MAX_PASSPHRASE {
        return Err("the passphrase is too long".into());
    }
    Ok(trimmed)
}

/// Reads a line from the terminal with echo off, restoring it afterwards.
fn prompt(text: &str) -> Result<String, String> {
    eprint!("{text}");
    let _ = io::stderr().flush();
    // SAFETY: A zeroed termios is a valid out value for `tcgetattr` on stdin.
    let mut original: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: Valid descriptor and out pointer.
    let has_terminal = unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut original) } == 0;
    if has_terminal {
        let mut quiet = original;
        quiet.c_lflag &= !libc::ECHO;
        // SAFETY: Valid descriptor and settings.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &quiet) };
    }
    let line = read_line(&mut io::stdin().lock());
    if has_terminal {
        // SAFETY: Restores the settings read above.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &original) };
        eprintln!();
    }
    line
}

pub fn print(reply: &Value) {
    if let Some(passphrase) = reply.get("passphrase").and_then(Value::as_str) {
        println!("Recovery passphrase: {passphrase}");
        println!("Write it down: another Mac needs it to join, and Dupin shows it only once.");
    }
    let on = reply.get("on").and_then(Value::as_bool).unwrap_or(false);
    println!("Sync: {}", if on { "on" } else { "off" });
    for (label, key) in [("Folder", "folder"), ("Key", "mode"), ("This Mac", "device")] {
        if let Some(value) = reply.get(key).and_then(Value::as_str) {
            println!("{label}: {value}");
        }
    }
    if let Some(problem) = reply.get("problem").and_then(Value::as_str) {
        println!("Problem: {problem}");
    }
    let Some(last) = reply.get("last").filter(|last| !last.is_null()) else {
        return;
    };
    for (flag, message) in [
        (
            "losing_vault",
            "Another vault in the folder wins; join it with `dupin sync join`.",
        ),
        (
            "newer_format",
            "A newer Dupin wrote to the folder; update to sync.",
        ),
        ("newer_epoch", "The vault key was rotated on another Mac."),
        ("reminted", "This Mac took a new device ID."),
    ] {
        if last.get(flag).and_then(Value::as_bool) == Some(true) {
            println!("{message}");
        }
    }
    for device in last
        .get("devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = device.get("name").and_then(Value::as_str).unwrap_or("?");
        let id = device.get("id").and_then(Value::as_str).unwrap_or("?");
        let mut notes = Vec::new();
        if device.get("inactive").and_then(Value::as_bool) == Some(true) {
            notes.push("inactive");
        }
        if device.get("retired").and_then(Value::as_bool) == Some(true) {
            notes.push("removed");
        }
        let notes = match notes.is_empty() {
            true => String::new(),
            false => format!(" ({})", notes.join(", ")),
        };
        println!("Other Mac: {name} {id}{notes}");
    }
    for (key, label) in [
        ("unreadable", "Unreadable"),
        ("clock_ahead", "Clock ahead"),
        ("blocked_adoption", "Waiting to take over"),
    ] {
        for item in last.get(key).and_then(Value::as_array).into_iter().flatten() {
            if let Some(item) = item.as_str() {
                println!("{label}: {item}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_one_bounded_line() {
        assert_eq!(
            read_line(&mut "secret words\nrest".as_bytes()).unwrap(),
            "secret words"
        );
        assert_eq!(read_line(&mut "crlf\r\n".as_bytes()).unwrap(), "crlf");
        let long = "x".repeat(MAX_PASSPHRASE + 10);
        assert!(read_line(&mut long.as_bytes()).is_err());
    }

    #[test]
    fn a_relative_folder_is_made_absolute_for_the_launcher() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            absolute(Some("Sync".into())).unwrap(),
            Some(cwd.join("Sync").display().to_string())
        );
        assert_eq!(absolute(Some("~/Sync".into())).unwrap(), Some("~/Sync".into()));
        assert_eq!(absolute(None).unwrap(), None);
    }
}
