use dupin_broker::{ConsentState, GrantSource};
use rune::{El, col, text};
use rune_kit::{Kit, Switch};

use super::Prefs;
use super::controls::{DESCRIPTION, ROW_PX, card, note, setting, setting_with_lock, tilde};
use crate::extensions::packages::{Kind, Listed};
use crate::icons;

pub(super) fn page(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let (listed, _) = &s.extensions;
    let mut body = vec![card(k, hair, vec![folder_setting(s, k)])];
    if !listed.is_empty() {
        body.push(card(
            k,
            hair,
            listed.iter().map(|listed| extension(k, listed)).collect(),
        ));
    }
    body.extend(problems(s, k));
    body.push(note(
        k,
        "Bundled extensions ship with Dupin and are on until you turn them off. \
         Run dupin ext eject NAME to copy one into the extensions folder and edit it there; \
         delete the copy to go back to the bundled one.",
    ));
    body
}

fn folder_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("extensions");
    let enabled = s.config.extensions;
    let switch = Switch::new(enabled)
        .label("Extensions folder")
        .hide_label()
        .disabled(locked)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.extensions = !enabled))
        .build(k);
    let folder = crate::extensions::directory()
        .map(|path| tilde(&path.to_string_lossy()))
        .unwrap_or_else(|| "the extensions folder".into());
    let status = if enabled {
        format!("Extensions in {folder} answer their keywords")
    } else {
        format!("Off. Extensions in {folder} are third-party code; when on, they answer their keywords")
    };
    setting_with_lock(
        k,
        &icons::FOLDER,
        "Extensions folder",
        Some(&status),
        switch,
        s.lock_note(locked),
    )
}

fn extension(k: &Kit, listed: &Listed) -> El {
    let (id, enabled) = (listed.id.clone(), listed.enabled);
    let switch = Switch::new(enabled)
        .label(listed.title.clone())
        .hide_label()
        .on_toggle(move |s: &mut Prefs| {
            let id = id.clone();
            s.panel
                .update(move |ui, tx| ui.set_extension_enabled(id, !enabled, tx));
        })
        .build(k);
    let origin = match listed.kind {
        Kind::Bundled => "Bundled with Dupin",
        Kind::Copy => "Your copy of a bundled extension, unchanged",
        Kind::Modified => "Your changed copy of a bundled extension, without its bundled grants",
        Kind::User => "From the extensions folder",
    };
    let mut detail = format!("{origin}. Type {} then Space.", listed.keyword);
    if !listed.scopes.is_empty() {
        let scopes: Vec<_> = listed
            .scopes
            .iter()
            .map(|(scope, state)| format!("{scope} ({})", standing(state)))
            .collect();
        detail.push_str(&format!(" Asks for {}.", scopes.join(", ")));
    }
    setting(k, &icons::PUZZLE, &listed.title, Some(&detail), switch).key(format!("extension-{}", listed.id))
}

fn standing(state: &ConsentState) -> &'static str {
    match state {
        ConsentState::Granted(GrantSource::Bundled) => "granted with Dupin",
        ConsentState::Granted(_) => "granted",
        ConsentState::Revoked => "revoked",
        ConsentState::DeniedByBase => "denied by your configuration",
        ConsentState::Changed => "changed; needs your consent",
        ConsentState::Undecided => "needs your consent",
    }
}

fn problems(s: &Prefs, k: &Kit) -> Option<El> {
    let (_, problems) = &s.extensions;
    (!problems.is_empty()).then(|| {
        col()
            .key("extension-problems")
            .w_full()
            .px(ROW_PX)
            .gap(4.0)
            .child(text("Skipped extensions").font(k.ty.ui).color(k.c.text_base))
            .children(problems.iter().map(|problem| {
                text(problem.clone())
                    .font(DESCRIPTION)
                    .color(k.c.text_muted)
                    .wrap()
                    .w_full()
            }))
    })
}
