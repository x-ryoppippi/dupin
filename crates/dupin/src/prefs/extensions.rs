//! Settings → Extensions: the extensions folder's switch, then each loaded package, bundled or
//! the user's, with its switch, its declared capabilities and where each stands, with Allow,
//! Revoke and Request Access. Opening the page only reads; macOS is asked only when the user
//! presses Request Access.

use dupin_extension_bridge::{
    Bridge, ConsentReason, ConsentRequest, ConsentState, GrantSource, OsAccess, OsPermission, PackageId,
    Scope, ScopeEntry,
};
use rune::{El, Icon, col, row, text};
use rune_kit::{Badge, BadgeVariant, Button, ButtonVariant, Kit, Switch};

use super::Prefs;
use super::controls::{DESCRIPTION, ROW_PX, card, note, setting, setting_with_lock, tilde};
use crate::extensions::packages::{Kind, Listed};
use crate::icons;

pub(super) fn page(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let mut rows = vec![card(k, hair, vec![folder_setting(s, k)])];
    rows.extend(skipped(s, k));
    let Some(bridge) = &s.access else {
        return rows;
    };
    let pending = bridge.pending(None);
    if !pending.is_empty() {
        rows.push(heading(k, "waiting", "Waiting for you"));
        rows.push(card(
            k,
            hair,
            pending
                .iter()
                .enumerate()
                .map(|(index, request)| pending_row(s, k, index, request))
                .collect(),
        ));
    }
    let (listed, _) = &s.extensions;
    for (index, extension) in listed.iter().enumerate() {
        rows.push(heading(
            k,
            &format!("extension-{index}"),
            &format!(
                "{} ({}{})",
                extension.title,
                extension.keyword,
                if extension.main_search {
                    ", in the main search"
                } else {
                    ""
                }
            ),
        ));
        rows.push(card(k, hair, package_rows(bridge, k, index, extension)));
    }
    for problem in bridge.problems() {
        rows.push(note(k, &problem));
    }
    rows.push(note(
        k,
        "An extension can use only what it declares and you allow here or when it asks. \
         Allowing runs nothing it asked for before. Revoking takes effect at once and drops \
         work it was doing.",
    ));
    rows.push(note(
        k,
        "Bundled extensions ship with Dupin and are on until you turn them off. \
         Run dupin ext eject NAME to copy one into the extensions folder and edit it there; \
         delete the copy to go back to the bundled one.",
    ));
    rows
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

/// Each folder that was skipped, with why.
fn skipped(s: &Prefs, k: &Kit) -> Option<El> {
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

fn heading(k: &Kit, key: &str, title: &str) -> El {
    text(title.to_owned())
        .key(key)
        .role("heading")
        .font(k.ty.ui_medium)
        .color(k.c.text_base)
        .mx(ROW_PX)
        .mt(8.0)
}

fn name_of(s: &Prefs, id: &PackageId) -> String {
    s.extensions
        .0
        .iter()
        .find(|extension| extension.package == *id)
        .map_or_else(|| id.to_string(), |extension| extension.title.clone())
}

fn pending_row(s: &Prefs, k: &Kit, index: usize, request: &ConsentRequest) -> El {
    let (id, revision, scope) = (
        request.package.clone(),
        request.revision.clone(),
        request.scope.clone(),
    );
    let title = format!(
        "{} wants to {}",
        name_of(s, &request.package),
        request.description
    );
    let detail = match request.reason {
        ConsentReason::OsAccess(permission) => {
            format!("macOS hasn't allowed Dupin {} yet.", permission.name())
        }
        ConsentReason::Changed => "An update changed what it asks for.".into(),
        ConsentReason::NotGranted => request.key().to_string(),
    };
    let allow = if let ConsentReason::OsAccess(permission) = request.reason {
        request_access_button(k, &format!("pending-{index}-request"), permission)
    } else {
        Button::new(if scope.needs_binding() {
            "Choose…"
        } else {
            "Allow"
        })
        .variant(ButtonVariant::Submit)
        .on_press(move |s: &mut Prefs| s.allow(&id, &revision, &scope))
        .build(k)
        .key(format!("pending-{index}-allow"))
    };
    let (deny_id, key) = (request.package.clone(), request.key());
    let deny = Button::new("Deny")
        .variant(ButtonVariant::Outline)
        .on_press(move |s: &mut Prefs| s.revoke(&deny_id, &key))
        .build(k)
        .key(format!("pending-{index}-deny"));
    setting(
        k,
        glyph(&request.scope),
        &title,
        Some(&detail),
        row().gap(8.0).center_y().child(deny).child(allow),
    )
}

fn package_rows(bridge: &Bridge, k: &Kit, index: usize, extension: &Listed) -> Vec<El> {
    let enabled = bridge.is_enabled(&extension.package);
    let runs = !bridge.is_turned_off(&extension.package);
    let id = extension.package.clone();
    let switch = Switch::new(enabled)
        .label(extension.title.clone())
        .hide_label()
        .on_toggle(move |s: &mut Prefs| s.set_extension_enabled(&id, !enabled))
        .build(k)
        .key(format!("extension-{index}-enabled"));
    let status = status(extension, enabled, runs);
    let mut rows = vec![setting(k, &icons::PUZZLE, "On", Some(&status), switch)];
    let scopes = bridge.scopes(&extension.package);
    if scopes.is_empty() {
        rows.push(setting(
            k,
            &icons::INFO,
            "Declares nothing",
            Some("It runs on what you type alone."),
            row(),
        ));
    }
    for (position, entry) in scopes.into_iter().enumerate() {
        rows.push(scope_row(
            bridge,
            k,
            &format!("extension-{index}-scope-{position}"),
            extension,
            entry,
        ));
    }
    rows
}

fn scope_row(bridge: &Bridge, k: &Kit, key: &str, extension: &Listed, entry: ScopeEntry) -> El {
    let ScopeEntry {
        scope,
        state,
        description,
        permission,
    } = entry;
    let (label, variant) = match state {
        ConsentState::Granted(GrantSource::Local) => ("Allowed", BadgeVariant::Accent),
        ConsentState::Granted(GrantSource::Base) => ("Allowed by config", BadgeVariant::Accent),
        ConsentState::Granted(GrantSource::Bundled) => ("Built in", BadgeVariant::Accent),
        ConsentState::Revoked => ("Revoked", BadgeVariant::Neutral),
        ConsentState::DeniedByBase => ("Denied by config", BadgeVariant::Neutral),
        ConsentState::Changed => ("Changed", BadgeVariant::Neutral),
        ConsentState::Undecided => ("Not allowed", BadgeVariant::Neutral),
    };
    let badge = Badge::new(label)
        .variant(variant)
        .build(k)
        .key(format!("{key}-state"));
    let mut controls = row().gap(8.0).center_y().child(badge);
    let id = extension.package.clone();
    let granted = matches!(state, ConsentState::Granted(_));
    if granted {
        let scope_key = scope.key();
        controls = controls.child(
            Button::new("Revoke")
                .variant(ButtonVariant::Outline)
                .on_press(move |s: &mut Prefs| s.revoke(&id, &scope_key))
                .build(k)
                .key(format!("{key}-revoke")),
        );
        if let Some(permission) = permission
            && bridge.os_access(permission) != OsAccess::Allowed
        {
            controls = controls.child(request_access_button(k, &format!("{key}-request"), permission));
        }
    } else if state != ConsentState::DeniedByBase
        && let Some(revision) = bridge.revision(&extension.package)
    {
        let scope_for_allow = scope.clone();
        controls = controls.child(
            Button::new(if scope.needs_binding() {
                "Choose…"
            } else {
                "Allow"
            })
            .on_press(move |s: &mut Prefs| s.allow(&id, &revision, &scope_for_allow))
            .build(k)
            .key(format!("{key}-allow")),
        );
    }
    let mut detail = scope.key().to_string();
    if let Some(permission) = permission {
        detail.push_str(&format!(" · Also needs macOS {} access", permission.name()));
    }
    for input in dupin_extension_bridge::exec_inputs(&scope) {
        detail.push_str(&format!(" · input {{{input}}} is one line of text"));
    }
    setting(
        k,
        glyph(&scope),
        &capitalized(&description),
        Some(&detail),
        controls,
    )
}

/// Where the extension came from, and whether its keyword reaches it and what it may use.
fn status(extension: &Listed, enabled: bool, runs: bool) -> String {
    let origin = match extension.kind {
        Kind::Bundled => "Bundled with Dupin",
        Kind::Copy => "Your copy of a bundled extension, unchanged",
        Kind::Modified => "Replaces a bundled extension of the same name, without its grants",
        Kind::User => "From the extensions folder",
    };
    let keyword = &extension.keyword;
    match (runs, extension.registered, enabled) {
        (false, ..) => format!("{origin}. Off."),
        (true, false, _) => format!("{origin}. Not answering: another command already uses {keyword}."),
        (true, true, true) => {
            format!("{origin}. Type {keyword} then Space; it can use what you allow below.")
        }
        (true, true, false) => {
            format!("{origin}. Type {keyword} then Space; it can use nothing below until you turn it on.")
        }
    }
}

fn request_access_button(k: &Kit, key: &str, permission: OsPermission) -> El {
    Button::new("Request Access")
        .on_press(move |s: &mut Prefs| s.request_os_access(permission))
        .build(k)
        .key(key.to_owned())
}

fn glyph(scope: &Scope) -> &'static Icon {
    match scope {
        Scope::Http { .. } => &icons::SEARCH,
        Scope::Files { .. } => &icons::FOLDER,
        Scope::Exec { .. } | Scope::Script { .. } => &icons::KEYBOARD,
        Scope::Storage { .. } => &icons::FILES,
        Scope::Clipboard { .. } => &icons::CLIPBOARD,
        Scope::Dictionary => &icons::LANGUAGES,
        Scope::Calendar { .. } => &icons::CALENDAR,
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// What Settings shows for a path a scope is bound to.
pub(super) fn shown_path(path: &std::path::Path) -> String {
    tilde(&path.to_string_lossy())
}

impl Prefs {
    pub fn reflect_extension_access(&mut self, bridge: Bridge) {
        self.access = Some(bridge);
    }

    /// The installed extensions that declare a scope needing `permission`, for its row in
    /// Settings → Permissions: each by name, marked when it holds no grant to use it.
    pub(super) fn extension_consumers(&self, permission: dupin_mac::privacy::Permission) -> Vec<String> {
        let Some(bridge) = &self.access else {
            return Vec::new();
        };
        let mut names: Vec<String> = bridge
            .os_consumers()
            .into_iter()
            .filter(|consumer| dupin_mac::privacy::Permission::of(consumer.permission) == permission)
            .map(|consumer| {
                let name = name_of(self, &consumer.package);
                if consumer.granted && consumer.enabled {
                    name
                } else {
                    format!("{name} (not allowed)")
                }
            })
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Opens Settings on Extensions, as a consent row's Open Settings does.
    pub fn show_extensions(&mut self) {
        self.show(super::Page::Extensions);
    }

    fn allow(&mut self, id: &PackageId, revision: &dupin_extension_bridge::Revision, scope: &Scope) {
        let Some(bridge) = self.access.clone() else { return };
        let binding = match scope {
            Scope::Files { name, kind, .. } => {
                let kind = match kind {
                    dupin_extension_bridge::FilesKind::File => dupin_mac::pick::Kind::File,
                    dupin_extension_bridge::FilesKind::Directory => dupin_mac::pick::Kind::Folder,
                };
                match pick(kind, &format!("Choose what “{name}” may read")) {
                    Some(path) => Some(path),
                    None => return,
                }
            }
            Scope::Exec { name, .. } => match pick(
                dupin_mac::pick::Kind::Program,
                &format!("Choose the program “{name}” runs"),
            ) {
                Some(path) => Some(path),
                None => return,
            },
            _ => None,
        };
        let picked = binding.as_deref().map(shown_path);
        let result = bridge.allow(id, revision, scope, binding);
        self.extension_problem = match result {
            Ok(()) => picked.map(|path| format!("Allowed {path}.")),
            Err(error) => Some(error.to_string()),
        };
    }

    fn revoke(&mut self, id: &PackageId, key: &dupin_extension_bridge::ScopeKey) {
        let Some(bridge) = self.access.clone() else { return };
        self.extension_problem = bridge.revoke(id, key).err().map(|error| error.to_string());
    }

    fn set_extension_enabled(&mut self, id: &PackageId, enabled: bool) {
        let Some(bridge) = self.access.clone() else { return };
        self.extension_problem = bridge
            .set_enabled(id, enabled)
            .err()
            .map(|error| error.to_string());
    }

    /// The deliberate press that may ask macOS, never anything Settings does on its own.
    fn request_os_access(&mut self, permission: OsPermission) {
        if cfg!(test) || rune::tooling::is_headless() {
            return;
        }
        match permission {
            OsPermission::Calendars => self.calendar_access.request(),
            OsPermission::Pasteboard => {
                std::thread::spawn(dupin_mac::clipboard::request_access);
            }
        }
        self.refresh_permissions();
    }
}

fn pick(kind: dupin_mac::pick::Kind, message: &str) -> Option<std::path::PathBuf> {
    if cfg!(test) || rune::tooling::is_headless() {
        return None;
    }
    dupin_mac::pick::choose(kind, message)
}

pub(super) fn problem(s: &Prefs, k: &Kit) -> Option<El> {
    s.extension_problem.as_deref().map(|problem| {
        col().key("extension-problem").w_full().px(ROW_PX).child(
            text(problem.to_owned())
                .font(DESCRIPTION)
                .color(k.c.text_base)
                .wrap(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_says_whether_its_keyword_reaches_it_and_what_it_may_use() {
        let listed = |registered| Listed {
            package: PackageId::new("help").unwrap(),
            title: "Help".into(),
            keyword: "help".into(),
            kind: Kind::User,
            registered,
            main_search: false,
        };
        assert_eq!(
            status(&listed(true), true, true),
            "From the extensions folder. Type help then Space; it can use what you allow below."
        );
        assert_eq!(
            status(&listed(true), false, true),
            "From the extensions folder. Type help then Space; it can use nothing below until you turn it on."
        );
        assert_eq!(
            status(&listed(false), true, true),
            "From the extensions folder. Not answering: another command already uses help."
        );
        assert_eq!(
            status(&listed(false), false, false),
            "From the extensions folder. Off."
        );
    }
}
