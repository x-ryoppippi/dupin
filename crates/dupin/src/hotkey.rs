//! Global shortcuts for the panel and configured targets.
//!
//! What can't be detected: another app holding the chord through a non-exclusive Carbon
//! registration (Raycast's, for one). Both registrations succeed and macOS delivers the key
//! to one of them, so the README asks users to unassign ⌘Space elsewhere first.

use std::collections::HashMap;
use std::sync::Mutex;

use dupin_core::{Config, config::HotkeyTarget};
use rune::{AppHandle, Shortcut, ShortcutError, ShortcutId};

use crate::ui::Ui;

pub struct Binding {
    panel: AppHandle<Ui>,
    state: Mutex<State>,
}

#[derive(Clone)]
pub enum Target {
    Panel,
    Configured(HotkeyTarget),
}

#[derive(Default)]
struct State {
    ids: HashMap<Shortcut, ShortcutId>,
    targets: HashMap<Shortcut, Target>,
    panel_unavailable: bool,
}

impl Binding {
    pub fn new(panel: AppHandle<Ui>) -> Binding {
        Binding {
            panel,
            state: Mutex::default(),
        }
    }

    pub fn failed_at_launch(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .panel_unavailable
    }

    /// Register the configured hotkey at launch, reporting any failure.
    pub fn register_at_launch(&self, accelerator: &str) {
        let shortcut = match parse(accelerator) {
            Ok(Some(shortcut)) => shortcut,
            Ok(None) => return,
            Err(e) => return self.report(accelerator, &e.to_string()),
        };
        // Carbon only refuses a chord another app grabbed exclusively. macOS's own shortcuts
        // (Spotlight's ⌘Space) take the key first without making registration fail, so ask who
        // holds it beforehand. Register anyway: freeing the chord later then works without a
        // restart.
        if let Some(owner) = self.panel.shortcut_owner(&shortcut) {
            self.report(accelerator, &format!("{owner} already uses it"));
        }
        match self.register(&shortcut) {
            Ok(id) => {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.ids.insert(shortcut.clone(), id);
                state.targets.insert(shortcut, Target::Panel);
                crate::trace::registered(accelerator);
            }
            Err(e) => self.report(accelerator, &self.why(&shortcut, &e)),
        }
    }

    /// Replace the panel shortcut recorded in Settings.
    pub fn replace(&self, next: Option<&Shortcut>) -> Result<(), ShortcutError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.panel_unavailable
            && next.is_some_and(|shortcut| matches!(state.targets.get(shortcut), Some(Target::Panel)))
        {
            return Err(ShortcutError::Conflict);
        }
        let mut targets = state.targets.clone();
        targets.retain(|_, target| !matches!(target, Target::Panel));
        if let Some(next) = next
            && targets.insert(next.clone(), Target::Panel).is_some()
        {
            return Err(ShortcutError::Conflict);
        }
        self.update_registrations(&mut state, &targets)
            .map_err(|(_, error)| error)?;
        state.targets = targets;
        state.panel_unavailable = false;
        Ok(())
    }

    pub fn apply_config(&self, config: &Config, previous: &Config) -> Result<(), String> {
        self.configure(config, previous, false)
    }

    pub fn reserve_for_save(&self, config: &Config, previous: &Config) -> Result<(), String> {
        self.configure(config, previous, true)
    }

    pub fn discard_reservations(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let targets = state.targets.clone();
        if let Err((shortcut, error)) = self.update_registrations(&mut state, &targets) {
            crate::log::warn(&format!("Couldn't release the {shortcut} shortcut: {error}."));
        }
    }

    fn configure(&self, config: &Config, previous: &Config, reserve: bool) -> Result<(), String> {
        let (panel, mut targets) = configured_targets(config)?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let keep_unavailable = state.panel_unavailable && parse(&previous.hotkey).ok() == Some(panel.clone());
        if keep_unavailable
            && let Some(panel) = panel
            && !state.ids.contains_key(&panel)
        {
            match self.claim(&panel) {
                Ok(id) => {
                    state.ids.insert(panel.clone(), id);
                    state.targets.insert(panel, Target::Panel);
                    state.panel_unavailable = false;
                }
                Err(_) => {
                    targets.remove(&panel);
                }
            }
        }
        // A pending disk save may fail. Hold its new chords without dispatching their actions.
        if reserve {
            targets.extend(state.targets.clone());
        }
        self.update_registrations(&mut state, &targets)
            .map_err(|(shortcut, error)| {
                format!(
                    "Couldn't apply {}: {}.",
                    shortcut.keycaps().join(""),
                    self.why(&shortcut, &error),
                )
            })?;
        if !reserve {
            state.targets = targets;
            state.panel_unavailable &= keep_unavailable;
        }
        Ok(())
    }

    pub fn target(&self, shortcut: &Shortcut) -> Option<Target> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .targets
            .get(shortcut)
            .cloned()
    }

    fn update_registrations(
        &self,
        state: &mut State,
        targets: &HashMap<Shortcut, Target>,
    ) -> Result<(), (Shortcut, ShortcutError)> {
        let register = |shortcut: &Shortcut| self.claim(shortcut);
        let unregister = |id| self.panel.unregister_shortcut(id);
        update_registrations(&mut state.ids, &state.targets, targets, register, unregister)
    }

    fn claim(&self, shortcut: &Shortcut) -> Result<ShortcutId, ShortcutError> {
        if self.owner(shortcut).is_some() {
            return Err(ShortcutError::Conflict);
        }
        self.register(shortcut)
    }

    fn register(&self, shortcut: &Shortcut) -> Result<ShortcutId, ShortcutError> {
        let shortcut = shortcut.clone();
        self.panel.register_shortcut(shortcut.clone(), move |ui, tx| {
            ui.activate_hotkey(&shortcut, tx);
        })
    }

    /// Who holds `shortcut`, for a refusal message.
    pub fn owner(&self, shortcut: &Shortcut) -> Option<String> {
        self.panel.shortcut_owner(shortcut)
    }

    fn report(&self, accelerator: &str, reason: &str) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .panel_unavailable = true;
        let message = format!(
            "Couldn't register the {} hotkey: {reason}. Free it in the app that holds it, or pick another in Settings (⌘,).",
            keycaps(accelerator)
        );
        crate::log::warn(&message);
        notify(&message);
        self.panel.update(move |s, tx| s.show_notice(message, tx));
    }

    fn why(&self, shortcut: &Shortcut, error: &ShortcutError) -> String {
        match (error, self.panel.shortcut_owner(shortcut)) {
            (ShortcutError::Conflict, Some(owner)) => format!("{owner} already uses it"),
            (ShortcutError::Conflict, None) => "another app already uses it".to_owned(),
            (e, _) => e.to_string(),
        }
    }
}

fn configured_targets(config: &Config) -> Result<(Option<Shortcut>, HashMap<Shortcut, Target>), String> {
    let panel = parse(&config.hotkey).map_err(|_| "The panel shortcut is invalid.")?;
    let mut targets = HashMap::new();
    if let Some(shortcut) = &panel {
        targets.insert(shortcut.clone(), Target::Panel);
    }
    for (accelerator, target) in &config.bindings {
        let shortcut = accelerator
            .parse::<Shortcut>()
            .map_err(|_| "A shortcut in [bindings] is invalid.")?;
        let HotkeyTarget::App { bundle_id, .. } = target;
        if bundle_id.is_empty() || bundle_id.contains(char::is_whitespace) {
            return Err(format!(
                "{} needs an app bundle identifier.",
                shortcut.keycaps().join("")
            ));
        }
        if targets
            .insert(shortcut.clone(), Target::Configured(target.clone()))
            .is_some()
        {
            return Err(format!(
                "{} is assigned more than once.",
                shortcut.keycaps().join("")
            ));
        }
    }
    Ok((panel, targets))
}

fn sync<Id: Copy>(
    ids: &mut HashMap<Shortcut, Id>,
    targets: &HashMap<Shortcut, Target>,
    register: impl Fn(&Shortcut) -> Result<Id, ShortcutError>,
    unregister: impl Fn(Id) -> Result<(), ShortcutError>,
) -> Result<(), (Shortcut, ShortcutError)> {
    // Keep current registrations until all new chords have been acquired. A chord that
    // changes target keeps its registration, so two bindings can swap on reload.
    for shortcut in targets.keys() {
        if let std::collections::hash_map::Entry::Vacant(slot) = ids.entry(shortcut.clone()) {
            let id = register(shortcut).map_err(|error| (shortcut.clone(), error))?;
            slot.insert(id);
        }
    }
    let removed: Vec<_> = ids
        .keys()
        .filter(|shortcut| !targets.contains_key(*shortcut))
        .cloned()
        .collect();
    for shortcut in removed {
        unregister(ids[&shortcut]).map_err(|error| (shortcut.clone(), error))?;
        ids.remove(&shortcut);
    }
    Ok(())
}

fn update_registrations<Id: Copy>(
    ids: &mut HashMap<Shortcut, Id>,
    previous: &HashMap<Shortcut, Target>,
    targets: &HashMap<Shortcut, Target>,
    register: impl Fn(&Shortcut) -> Result<Id, ShortcutError>,
    unregister: impl Fn(Id) -> Result<(), ShortcutError>,
) -> Result<(), (Shortcut, ShortcutError)> {
    if let Err(error) = sync(ids, targets, &register, &unregister) {
        if let Err((shortcut, error)) = sync(ids, previous, register, unregister) {
            crate::log::warn(&format!("Couldn't restore the {shortcut} shortcut: {error}."));
        }
        return Err(error);
    }
    Ok(())
}

pub fn parse(accelerator: &str) -> Result<Option<Shortcut>, ShortcutError> {
    if accelerator.trim().is_empty() {
        Ok(None)
    } else {
        accelerator.parse().map(Some)
    }
}

fn notify(message: &str) {
    if rune_notify::available() {
        rune_notify::post(
            &rune_notify::Notification::new("hotkey", "Dupin hotkey unavailable").body(message),
        );
    }
}

/// `cmd+space` as macOS writes it: ⌘Space.
pub fn keycaps(accelerator: &str) -> String {
    let mut out = String::new();
    for part in accelerator.split('+').map(str::trim) {
        out.push_str(match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" | "mod" => "⌘",
            "alt" | "option" | "opt" => "⌥",
            "ctrl" | "control" => "⌃",
            "shift" => "⇧",
            "space" => "Space",
            _ => {
                out.push_str(&part.to_uppercase());
                continue;
            }
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn app() -> HotkeyTarget {
        HotkeyTarget::App {
            bundle_id: "com.apple.calculator".into(),
            hide_if_frontmost: true,
        }
    }

    #[test]
    fn rejects_aliases_of_the_panel_or_another_binding() {
        let mut config = Config::default();
        config.bindings.insert("Command+Space".into(), app());
        assert!(configured_targets(&config).is_err());
        config.hotkey.clear();
        config.bindings.insert("cmd+space".into(), app());
        assert!(configured_targets(&config).is_err());
    }

    #[test]
    fn can_reassign_an_installed_chord_and_release_a_removed_chord() {
        let panel: Shortcut = "cmd+space".parse().unwrap();
        let app_key: Shortcut = "ctrl+alt+c".parse().unwrap();
        let mut ids = HashMap::from([(panel.clone(), 1), (app_key.clone(), 2)]);
        let previous = HashMap::from([
            (panel.clone(), Target::Panel),
            (app_key, Target::Configured(app())),
        ]);
        let next = HashMap::from([(panel.clone(), Target::Configured(app()))]);

        update_registrations(
            &mut ids,
            &previous,
            &next,
            |_| panic!("already registered"),
            |id| {
                assert_eq!(id, 2);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(ids, HashMap::from([(panel, 1)]));
    }

    #[test]
    fn a_conflict_releases_partially_registered_shortcuts() {
        let (panel, previous) = configured_targets(&Config::default()).unwrap();
        let panel = panel.unwrap();
        let mut ids = HashMap::from([(panel.clone(), 1)]);
        let next = HashMap::from([
            ("ctrl+alt+c".parse().unwrap(), Target::Configured(app())),
            ("ctrl+alt+d".parse().unwrap(), Target::Configured(app())),
        ]);
        let attempted = Cell::new(false);
        let release = Cell::new(None);
        let error = update_registrations(
            &mut ids,
            &previous,
            &next,
            |_| {
                if attempted.replace(true) {
                    Err(ShortcutError::Conflict)
                } else {
                    Ok(2)
                }
            },
            |id| {
                assert_eq!(release.replace(Some(id)), None);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.1, ShortcutError::Conflict);
        assert_eq!(release.get(), Some(2));
        assert_eq!(ids, HashMap::from([(panel, 1)]));
    }

    #[test]
    fn an_empty_hotkey_disables_registration() {
        assert_eq!(super::parse(""), Ok(None));
        assert_eq!(super::parse("  "), Ok(None));
        assert!(super::parse("cmd+not-a-key").is_err());
        assert!(super::parse("cmd+space").unwrap().is_some());
    }

    #[test]
    fn equivalent_spellings_describe_the_same_hotkey() {
        assert_eq!(super::parse("cmd+space"), super::parse("Command+Space"));
    }

    #[test]
    fn a_recorded_shortcut_is_saved_in_a_form_that_reads_back() {
        for accelerator in ["cmd+space", "ctrl+space", "ctrl+alt+k", "shift+cmd+p"] {
            let shortcut: rune::Shortcut = accelerator.parse().unwrap();
            let saved = shortcut.to_string();
            assert_eq!(
                saved.parse::<rune::Shortcut>().unwrap(),
                shortcut,
                "{accelerator} saved as {saved}"
            );
        }
    }

    #[test]
    fn writes_modifiers_as_macos_glyphs() {
        assert_eq!(super::keycaps("cmd+space"), "⌘Space");
        assert_eq!(super::keycaps("ctrl+alt+k"), "⌃⌥K");
    }
}
