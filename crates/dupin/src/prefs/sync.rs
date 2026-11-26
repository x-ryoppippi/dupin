//! Settings → Sync: turn sync on in iCloud Drive or a chosen folder, join another Mac's, see
//! the other Macs and how sync is going, and turn it off. Every action goes to the sync worker
//! in the panel's process; nothing here touches the folder or the Keychain.

use std::path::PathBuf;

use rune::{El, col, text};
use rune_kit::{Button, Kit};
use serde_json::Value;
use zeroize::Zeroizing;

use super::controls::{card, note, setting, tilde, value_button};
use super::{DESCRIPTION, Prefs, ROW_PX};
use crate::control::SyncAction;
use crate::icons;

#[derive(Default)]
pub(super) struct SyncPane {
    /// The worker's last status, `dupin sync status --json`'s shape.
    status: Option<Value>,
    busy: bool,
    problem: Option<String>,
    /// The folder chosen for Turn On Sync or Join, before sync is on.
    folder: Option<PathBuf>,
    /// The generated passphrase, shown until the user says it is written down.
    passphrase: Option<Zeroizing<String>>,
}

impl Prefs {
    pub(super) fn sync_ask(&mut self, action: SyncAction) {
        self.sync.busy = !matches!(action, SyncAction::Status {});
        self.sync.problem = None;
        self.panel.update(move |ui, _| ui.sync_request(action));
    }

    /// The worker's answer to an action from this page.
    pub(crate) fn sync_answered(&mut self, result: Result<Value, String>) {
        self.sync.busy = false;
        match result {
            Ok(mut status) => {
                if let Some(Value::String(words)) = status.get_mut("passphrase").map(Value::take) {
                    self.sync.passphrase = Some(Zeroizing::new(words));
                }
                if let Some(object) = status.as_object_mut() {
                    object.remove("passphrase");
                }
                self.sync.status = Some(status);
            }
            Err(problem) => self.sync.problem = Some(problem),
        }
    }

    fn sync_folder_arg(&self) -> Option<String> {
        self.sync
            .folder
            .as_ref()
            .map(|folder| folder.display().to_string())
    }
}

fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn ago(at_ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    let minutes = now.saturating_sub(at_ms) / 60_000;
    match minutes {
        0 => "just now".into(),
        1 => "a minute ago".into(),
        2..=59 => format!("{minutes} minutes ago"),
        60..=119 => "an hour ago".into(),
        120..=2879 => format!("{} hours ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}

pub(super) fn page(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let mut body = Vec::new();
    if let Some(problem) = &s.sync.problem {
        body.push(
            text(problem.clone())
                .key("sync-problem")
                .w_full()
                .mx(ROW_PX)
                .font(DESCRIPTION)
                .color(k.c.text_base)
                .wrap(),
        );
    }
    if let Some(words) = &s.sync.passphrase {
        body.push(card(
            k,
            hair,
            vec![setting(
                k,
                &icons::SHIELD,
                "Recovery passphrase",
                Some(&format!(
                    "{}. Write it down: another Mac needs it to join, and Dupin shows it only once.",
                    words.as_str()
                )),
                Button::new("I've Written It Down")
                    .on_press(|s: &mut Prefs| s.sync.passphrase = None)
                    .build(k),
            )],
        ));
    }
    let Some(status) = &s.sync.status else {
        body.push(note(k, "Checking sync…"));
        return body;
    };
    let on = status.get("on").and_then(Value::as_bool) == Some(true);
    let leaving = status.get("leaving").and_then(Value::as_bool) == Some(true);
    if on || leaving {
        body.extend(on_page(s, k, hair, status, leaving));
    } else {
        body.extend(off_page(s, k, hair, status));
    }
    body
}

fn off_page(s: &Prefs, k: &Kit, hair: f32, status: &Value) -> Vec<El> {
    let folder = s
        .sync
        .folder
        .as_ref()
        .map(|folder| folder.display().to_string())
        .or_else(|| str_of(status, "folder").map(str::to_owned))
        .or_else(|| {
            std::env::var_os("HOME")
                .and_then(|home| dupin_sync::icloud_drive_default(std::path::Path::new(&home)))
                .map(|path| path.display().to_string())
        });
    let folder_line = folder.as_deref().map_or_else(
        || "Choose a folder that iCloud Drive, Dropbox or another tool syncs".to_owned(),
        tilde,
    );
    let busy = s.sync.busy;
    let rows = vec![
        setting(
            k,
            &icons::FOLDER,
            "Sync folder",
            Some(&folder_line),
            value_button(k, "sync-choose-folder", "Choose Folder…", &icons::FOLDER).on_press(
                |s: &mut Prefs| {
                    if let Some(folder) = super::extensions::pick(
                        dupin_mac::pick::Kind::Folder,
                        "Choose the folder to sync Dupin through",
                    ) {
                        s.sync.folder = Some(folder);
                    }
                },
            ),
        ),
        setting(
            k,
            &icons::CLOUD,
            "Turn On Sync",
            Some("Creates an encrypted vault in the folder, with a passphrase Dupin makes for you"),
            Button::new("Turn On Sync")
                .disabled(busy || folder.is_none())
                .on_press(|s: &mut Prefs| {
                    let folder = s.sync_folder_arg();
                    s.sync_ask(SyncAction::On {
                        folder,
                        passphrase: None,
                    });
                })
                .build(k),
        ),
        setting(
            k,
            &icons::SHIELD,
            "Use your own passphrase",
            Some("At least 12 characters, and not a commonly used password"),
            Button::new("Turn On With Passphrase…")
                .disabled(busy || folder.is_none())
                .on_press(|s: &mut Prefs| {
                    if cfg!(test) || rune::tooling::is_headless() {
                        return;
                    }
                    let ask = |message: &str| {
                        dupin_mac::secret_prompt::ask(
                            message,
                            "Another Mac needs this passphrase to join. Dupin never stores it.",
                            "Continue",
                        )
                    };
                    let Some(first) = ask("Choose a sync passphrase") else {
                        return;
                    };
                    let Some(again) = ask("Enter the passphrase again") else {
                        return;
                    };
                    if first != again {
                        s.sync.problem = Some("The two passphrases differ; nothing was changed.".into());
                        return;
                    }
                    let folder = s.sync_folder_arg();
                    s.sync_ask(SyncAction::On {
                        folder,
                        passphrase: Some(first.to_string()),
                    });
                })
                .build(k),
        ),
        setting(
            k,
            &icons::LAPTOP,
            "Join another Mac",
            Some("Uses the vault another Mac made in the folder, with its passphrase"),
            Button::new("Join…")
                .disabled(busy || folder.is_none())
                .on_press(|s: &mut Prefs| {
                    if cfg!(test) || rune::tooling::is_headless() {
                        return;
                    }
                    let typed = dupin_mac::secret_prompt::ask(
                        "Join sync",
                        "Enter the recovery passphrase shown on the Mac that turned sync on. Leave it empty to use iCloud Keychain.",
                        "Join",
                    );
                    let folder = s.sync_folder_arg();
                    s.sync_ask(SyncAction::Join {
                        folder,
                        passphrase: typed.map(|typed| typed.to_string()),
                    });
                })
                .build(k),
        ),
    ];
    vec![
        card(k, hair, rows),
        note(
            k,
            "Sync keeps your Macs' usage history in step through the folder, end-to-end encrypted: the folder's host sees no contents and no names. Settings, snippets and extensions follow.",
        ),
    ]
}

fn on_page(s: &Prefs, k: &Kit, hair: f32, status: &Value, leaving: bool) -> Vec<El> {
    let last = status.get("last").filter(|last| !last.is_null());
    let synced = last
        .and_then(|last| last.get("at"))
        .and_then(Value::as_u64)
        .map_or_else(|| "Not yet".to_owned(), ago);
    let key = match str_of(status, "mode") {
        Some("icloud-keychain") => "iCloud Keychain, with the recovery passphrase as a backup",
        Some(_) => "Recovery passphrase",
        None => "Folder unavailable",
    };
    let busy = s.sync.busy;
    let mut rows = vec![
        setting(
            k,
            &icons::FOLDER,
            "Sync folder",
            Some(&str_of(status, "folder").map_or_else(String::new, tilde)),
            col(),
        ),
        setting(k, &icons::SHIELD, "Key", Some(key), col()),
        setting(
            k,
            &icons::REFRESH,
            "Last synced",
            Some(&synced),
            Button::new("Sync Now")
                .disabled(busy || leaving)
                .on_press(|s: &mut Prefs| s.sync_ask(SyncAction::Now {}))
                .build(k),
        ),
    ];
    if leaving {
        rows.push(setting(
            k,
            &icons::INFO,
            "Turning off",
            Some("The other Macs are taking over what only this one held"),
            col(),
        ));
    } else {
        rows.push(setting(
            k,
            &icons::CLOUD,
            "Turn Off Sync",
            Some("This Mac stops syncing; its data stays here"),
            Button::new("Turn Off")
                .disabled(busy)
                .on_press(|s: &mut Prefs| s.sync_ask(SyncAction::Off {}))
                .build(k),
        ));
    }
    let mut body = vec![card(k, hair, rows)];

    let devices: Vec<&Value> = last
        .and_then(|last| last.get("devices"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|device| device.get("retired").and_then(Value::as_bool) != Some(true))
        .collect();
    if !devices.is_empty() {
        let rows = devices
            .into_iter()
            .map(|device| {
                let id = str_of(device, "id").unwrap_or_default().to_owned();
                let mut detail = device
                    .get("seen_at")
                    .and_then(Value::as_u64)
                    .map_or_else(String::new, |at| format!("Last synced {}", ago(at)));
                if device.get("inactive").and_then(Value::as_bool) == Some(true) {
                    detail.push_str(" · inactive for over 90 days");
                }
                setting(
                    k,
                    &icons::LAPTOP,
                    str_of(device, "name").unwrap_or("Mac"),
                    Some(&detail),
                    Button::new("Remove This Mac")
                        .disabled(busy)
                        .on_press(move |s: &mut Prefs| {
                            s.sync_ask(SyncAction::RemoveDevice { device: id.clone() })
                        })
                        .build(k),
                )
            })
            .collect();
        body.push(card(k, hair, rows));
    }
    for line in problems(status) {
        body.push(note(k, &line));
    }
    body
}

/// What the last pass found wrong, in words.
fn problems(status: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(problem) = str_of(status, "problem") {
        lines.push(problem.to_owned());
    }
    let Some(last) = status.get("last").filter(|last| !last.is_null()) else {
        return lines;
    };
    let flag = |key: &str| last.get(key).and_then(Value::as_bool) == Some(true);
    if flag("losing_vault") {
        lines.push("Another Mac made a vault in the same folder at the same time; it wins. Turn sync off and join it.".into());
    }
    if flag("newer_format") {
        lines.push("A newer version of Dupin wrote to the folder. Update Dupin to keep syncing.".into());
    }
    if flag("newer_epoch") {
        lines.push("The sync key was changed on another Mac.".into());
    }
    let count = |key: &str| last.get(key).and_then(Value::as_array).map_or(0, Vec::len);
    if count("unreadable") > 0 {
        lines.push(format!(
            "{} files in the folder didn't open; Dupin retries them and never deletes them.",
            count("unreadable")
        ));
    }
    if count("clock_ahead") > 0 {
        lines.push(
            "Another Mac's clock is more than five minutes ahead; its changes wait until the clocks agree."
                .into(),
        );
    }
    if count("blocked_adoption") > 0 {
        lines.push(
            "A Mac that left has files that don't open yet, so the others can't take them over.".into(),
        );
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn problems_are_put_in_words() {
        let status = json!({
            "problem": "folder unavailable: no vault",
            "last": {"losing_vault": true, "unreadable": ["ns/x/y.rec"], "clock_ahead": []},
        });
        let lines = problems(&status);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].starts_with("folder unavailable"));
        assert!(problems(&json!({"last": null})).is_empty());
    }

    #[test]
    fn times_read_as_ago() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert_eq!(ago(now), "just now");
        assert_eq!(ago(now - 5 * 60_000), "5 minutes ago");
        assert_eq!(ago(now - 3 * 86_400_000), "3 days ago");
    }
}
