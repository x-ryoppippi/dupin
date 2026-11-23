use rune::{El, col};
use rune_kit::{Button, Kit, Switch};

use super::controls::{card, setting, setting_with_lock};
use super::{Page, Prefs, footer_link};
use crate::icons;
use crate::update::Status;

/// Whether to install updates by themselves, and where the updater is: the version, what it
/// is doing, and the one thing to do next.
pub(super) fn page(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let locked = s.locked("auto_update");
    let next_auto_update = !s.config.auto_update;
    let auto = Switch::new(s.config.auto_update)
        .label("Install updates automatically")
        .hide_label()
        .disabled(locked)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.auto_update = next_auto_update))
        .build(k);
    let status = s.updater.status();
    let action = match &status {
        Status::Ready(_) => Button::new("Restart to Update").on_press(|s: &mut Prefs| {
            s.panel.update(|ui, tx| ui.relaunch(tx));
        }),
        Status::Download { .. } => {
            Button::new("Download").on_press(|s: &mut Prefs| s.updater.open_download())
        }
        _ => Button::new("Check for Updates")
            .disabled(!status.can_check())
            .on_press(|s: &mut Prefs| s.updater.check_now()),
    }
    .build(k);
    let mut line = status.text(s.config.auto_update, s.updater.download_only());
    if matches!(status, Status::Ready(_)) {
        line.push_str(". Restarting cancels running scripts.");
    }
    let rows = vec![
        setting_with_lock(
            k,
            &icons::REFRESH,
            "Install updates automatically",
            Some("New versions download in the background and install on restart"),
            auto,
            locked,
        ),
        setting(
            k,
            &icons::INFO,
            &format!("Dupin {}", s.updater.version()),
            Some(&line),
            action,
        ),
    ];
    vec![card(k, hair, rows)]
}

pub(super) fn footer(s: &Prefs, k: &Kit) -> El {
    let status = s.updater.status();
    let title = match &status {
        Status::Off => "Update settings",
        Status::Checking => "Checking…",
        Status::Downloading => "Downloading…",
        Status::Ready(_) => "Update ready",
        Status::Download { .. } => "Update available",
        _ => "Check for Updates",
    };
    let version = if s.updater.version().is_empty() {
        "Updates".to_owned()
    } else {
        format!("Version {}", s.updater.version())
    };
    col()
        .key("update-footer")
        .w_full()
        .mt(12.0)
        .child(
            footer_link(k, "current-version", &version)
                .label(format!("{version}. Open update settings"))
                .focus_selected(s.page == Page::Updates)
                .w_full()
                .on_press(|s: &mut Prefs| s.show(Page::Updates)),
        )
        .child(
            footer_link(k, Page::Updates.key(), title)
                .w_full()
                .h(24.0)
                .on_press(|s: &mut Prefs| {
                    s.show(Page::Updates);
                    let status = s.updater.status();
                    if status.can_check() && !matches!(status, Status::Download { .. }) {
                        s.updater.check_now();
                    }
                }),
        )
}
