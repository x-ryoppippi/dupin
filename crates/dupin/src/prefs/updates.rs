use rune::{El, col, row, text};
use rune_kit::{Button, Kit, Switch};

use super::controls::{card, setting};
use super::{Page, Prefs};
use crate::icons;
use crate::update::Status;

/// Whether to install updates by themselves, and where the updater is: the version, what it
/// is doing, and the one thing to do next.
pub(super) fn page(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let next_auto_update = !s.config.auto_update;
    let auto = Switch::new(s.config.auto_update)
        .label("Install updates automatically")
        .hide_label()
        .disabled(s.read_only)
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
        setting(
            k,
            &icons::REFRESH,
            "Install updates automatically",
            Some("New versions download in the background and install on restart"),
            auto,
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
            text(version.clone())
                .key("current-version")
                .role("button")
                .label(format!("{version}. Open update settings"))
                .focusable(true)
                .focus_selected(s.page == Page::Updates)
                .focus_ring(2.0, 0.0, k.c.border_focus)
                .on_press(|s: &mut Prefs| s.show(Page::Updates))
                .w_full()
                .px(6.0)
                .font(k.ty.small.lh(20.0))
                .color(k.c.text_muted)
                .truncate(),
        )
        .child(
            row()
                .key(Page::Updates.key())
                .role("button")
                .label(title)
                .focusable(true)
                .focus_ring(2.0, 0.0, k.c.border_focus)
                .w_full()
                .h(24.0)
                .px(6.0)
                .center_y()
                .radius(k.radius.sm)
                .hover(k.c.hover)
                .press(k.c.pressed)
                .on_press(|s: &mut Prefs| {
                    s.show(Page::Updates);
                    let status = s.updater.status();
                    if status.can_check() && !matches!(status, Status::Download { .. }) {
                        s.updater.check_now();
                    }
                })
                .child(text(title).font(k.ty.small).color(k.c.text_muted).truncate()),
        )
}
