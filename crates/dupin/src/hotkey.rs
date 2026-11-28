//! Global shortcuts for the panel and configured targets.
//! On a first run, the welcome in Settings explains panel shortcut registration failures.
//!
//! What can't be detected: another app holding the chord through a non-exclusive Carbon
//! registration (Raycast's, for one). Both registrations succeed and macOS delivers the key
//! to one of them, so the README asks users to unassign ⌘Space elsewhere first.

use std::collections::HashMap;
use std::sync::Mutex;

use dupin_core::Config;
use dupin_core::config::{Address, HotkeyTarget};
use dupin_mac::system_hotkey;
use rune::{AppHandle, Shortcut, ShortcutError, ShortcutId};

use crate::{scripts::catalog::Catalog, ui::Ui};

pub struct Binding {
    panel: AppHandle<Ui>,
    state: Mutex<State>,
}

#[derive(Clone)]
pub enum Target {
    Launcher(LauncherShortcut),
    Configured(HotkeyTarget),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LauncherShortcut {
    Panel,
    Selection,
}

impl LauncherShortcut {
    pub const ALL: [Self; 2] = [Self::Panel, Self::Selection];

    pub fn accelerator(self, config: &Config) -> &str {
        match self {
            Self::Panel => &config.hotkey,
            Self::Selection => &config.selection_hotkey,
        }
    }

    pub fn set(self, config: &mut Config, shortcut: Option<&Shortcut>) {
        let field = match self {
            Self::Panel => &mut config.hotkey,
            Self::Selection => &mut config.selection_hotkey,
        };
        *field = shortcut.map(ToString::to_string).unwrap_or_default();
    }

    fn name(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Selection => "selection",
        }
    }
}

#[derive(Default)]
struct State {
    ids: HashMap<Shortcut, ShortcutId>,
    targets: HashMap<Shortcut, Target>,
    launch_failures: [Option<LaunchFailure>; 2],
}

struct LaunchFailure {
    shortcut: Option<Shortcut>,
    problem: LaunchProblem,
}

#[derive(Clone)]
pub enum LaunchProblem {
    Occupied(String),
    Registration(ShortcutError),
    AssignedToPanel,
}

impl LaunchProblem {
    fn reason(&self) -> String {
        match self {
            Self::Occupied(owner) => format!("{owner} already uses it"),
            Self::Registration(ShortcutError::Conflict) => "another app already uses it".into(),
            Self::Registration(error) => error.to_string(),
            Self::AssignedToPanel => "already assigned to the panel".into(),
        }
    }
}

impl Binding {
    pub fn new(panel: AppHandle<Ui>) -> Binding {
        Binding {
            panel,
            state: Mutex::default(),
        }
    }

    pub fn launch_problem(
        &self,
        kind: LauncherShortcut,
        shortcut: Option<&Shortcut>,
    ) -> Option<LaunchProblem> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let failure = state.launch_failures[kind as usize].as_ref()?;
        (failure.shortcut.as_ref() == shortcut).then(|| failure.problem.clone())
    }

    pub fn refresh_launch_conflict(&self, kind: LauncherShortcut) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let installed = state.targets.iter().find_map(|(shortcut, target)| {
            matches!(target, Target::Launcher(found) if *found == kind).then(|| shortcut.clone())
        });
        refresh_system_conflict(
            &mut state.launch_failures[kind as usize],
            installed.as_ref(),
            system_hotkey::owner,
        )
    }

    /// Without `announce`, leave it to the welcome to present the failure.
    pub fn register_at_launch(&self, config: &Config, announce: bool) {
        let problems: Vec<_> = LauncherShortcut::ALL
            .into_iter()
            .filter_map(|kind| {
                let accelerator = kind.accelerator(config);
                let failure = self.register_launcher(kind, accelerator).err()?;
                let reason = failure.problem.reason();
                self.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .launch_failures[kind as usize] = Some(failure);
                Some(format!(
                    "Couldn't register the {} hotkey ({}): {reason}.",
                    kind.name(),
                    keycaps(accelerator)
                ))
            })
            .collect();
        if !problems.is_empty() {
            let message = format!(
                "{} Free the shortcut in the app that holds it, or pick another in Settings (⌘,).",
                problems.join(" ")
            );
            crate::log::warn(&message);
            if announce {
                notify(&message);
                self.panel.update(move |s, tx| s.show_notice(message, tx));
            }
        }
    }

    fn register_launcher(&self, kind: LauncherShortcut, accelerator: &str) -> Result<(), LaunchFailure> {
        let Some(shortcut) = parse(accelerator).map_err(|error| LaunchFailure {
            shortcut: None,
            problem: LaunchProblem::Registration(error),
        })?
        else {
            return Ok(());
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.ids.contains_key(&shortcut) {
            return Err(LaunchFailure {
                shortcut: Some(shortcut),
                problem: LaunchProblem::AssignedToPanel,
            });
        }
        // Carbon only refuses a chord another app grabbed exclusively. macOS's own shortcuts
        // (Spotlight's ⌘Space) take the key first without making registration fail, so ask who
        // holds it beforehand. Register anyway: freeing the chord later then works without a
        // restart.
        let owner = self.owner(&shortcut);
        let problem = match self.register(&shortcut) {
            Ok(id) => {
                state.ids.insert(shortcut.clone(), id);
                state.targets.insert(shortcut.clone(), Target::Launcher(kind));
                crate::trace::registered(accelerator);
                let Some(owner) = owner else { return Ok(()) };
                LaunchProblem::Occupied(owner)
            }
            Err(ShortcutError::Conflict) => owner.or_else(|| self.owner(&shortcut)).map_or(
                LaunchProblem::Registration(ShortcutError::Conflict),
                LaunchProblem::Occupied,
            ),
            Err(error) => LaunchProblem::Registration(error),
        };
        Err(LaunchFailure {
            shortcut: Some(shortcut),
            problem,
        })
    }

    pub fn replace(&self, kind: LauncherShortcut, next: Option<&Shortcut>) -> Result<(), ShortcutError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let targets = replacement_targets(&state, kind, next, |shortcut| {
            system_hotkey::owner(shortcut).is_some()
        })?;
        self.update_registrations(&mut state, &targets)
            .map_err(|(_, error)| error)?;
        state.targets = targets;
        state.launch_failures[kind as usize] = None;
        Ok(())
    }

    pub fn apply_config(
        &self,
        config: &Config,
        previous: &Config,
        scripts: Option<&Catalog>,
        commands: Commands,
    ) -> Result<Option<String>, String> {
        self.configure(config, previous, (scripts, commands), false)
    }

    pub fn reserve_for_save(
        &self,
        config: &Config,
        previous: &Config,
        scripts: Option<&Catalog>,
        commands: Commands,
    ) -> Result<(), String> {
        self.configure(config, previous, (scripts, commands), true)
            .map(|_| ())
    }

    pub fn discard_reservations(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let targets = state.targets.clone();
        if let Err((shortcut, error)) = self.update_registrations(&mut state, &targets) {
            crate::log::warn(&format!("Couldn't release the {shortcut} shortcut: {error}."));
        }
    }

    fn configure(
        &self,
        config: &Config,
        previous: &Config,
        (scripts, commands): (Option<&Catalog>, Commands),
        reserve: bool,
    ) -> Result<Option<String>, String> {
        let mut targets = configured_targets(config)?;
        let mut problems = filter_unavailable(&mut targets, scripts, commands);
        let previous_bindings = binding_targets(previous);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut keep_unavailable = [false; 2];
        for kind in LauncherShortcut::ALL {
            let index = kind as usize;
            if state.launch_failures[index].is_none() {
                continue;
            }
            let next = parse(kind.accelerator(config)).ok().flatten();
            if next != parse(kind.accelerator(previous)).ok().flatten() {
                continue;
            }
            let Some(shortcut) = next else { continue };
            keep_unavailable[index] = true;
            if state.ids.contains_key(&shortcut) {
                continue;
            }
            match self.claim(&shortcut) {
                Ok(id) => {
                    state.ids.insert(shortcut.clone(), id);
                    state.targets.insert(shortcut, Target::Launcher(kind));
                    state.launch_failures[index] = None;
                    keep_unavailable[index] = false;
                }
                Err(_) => {
                    targets.remove(&shortcut);
                }
            }
        }
        // A pending disk save may fail. Hold its new chords without dispatching their actions.
        if reserve {
            targets.extend(state.targets.clone());
        }
        let register = |shortcut: &Shortcut| self.claim(shortcut);
        let unregister = |id| self.panel.unregister_shortcut(id);
        let skip = |shortcut: &Shortcut, error: &ShortcutError| {
            if *error != ShortcutError::Conflict || !unchanged_binding(shortcut, &targets, &previous_bindings)
            {
                return false;
            }
            problems.push(format!(
                "Couldn't register the {} binding: {}.",
                shortcut.keycaps().join(""),
                self.why(shortcut, error),
            ));
            true
        };
        let State {
            ids, targets: active, ..
        } = &mut *state;
        sync_transaction(ids, active, &targets, register, unregister, skip).map_err(
            |(shortcut, error)| {
                format!(
                    "Couldn't apply {}: {}.",
                    shortcut.keycaps().join(""),
                    self.why(&shortcut, &error)
                )
            },
        )?;
        if !reserve {
            targets.retain(|shortcut, _| state.ids.contains_key(shortcut));
            state.targets = targets;
            for (failure, keep) in state.launch_failures.iter_mut().zip(keep_unavailable) {
                if !keep {
                    *failure = None;
                }
            }
        }
        problems.sort_unstable();
        Ok((!problems.is_empty()).then(|| problems.join(" ")))
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

    fn owner(&self, shortcut: &Shortcut) -> Option<String> {
        self.panel.shortcut_owner(shortcut)
    }

    pub fn recorder_conflict(
        &self,
        kind: LauncherShortcut,
        shortcut: &Shortcut,
        config: &Config,
    ) -> Option<String> {
        if let Some(key) = crate::list_keys::holding(config, shortcut) {
            return Some(format!(
                "{} is assigned to the {}.",
                shortcut.keycaps().join(""),
                key.name()
            ));
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(state.targets.get(shortcut), Some(Target::Configured(_)))
            || binding_targets(config).contains_key(shortcut)
        {
            return Some(format!(
                "{} is assigned to a binding in config.toml.",
                shortcut.keycaps().join("")
            ));
        }
        if let Some(Target::Launcher(other)) = state.targets.get(shortcut) {
            let keys = shortcut.keycaps().join("");
            return Some(if *other == kind {
                let owner = system_hotkey::owner(shortcut)?;
                format!("{keys} is used by {owner}.")
            } else {
                format!("{keys} is assigned to the {} shortcut.", other.name())
            });
        }
        self.owner(shortcut)
            .map(|owner| format!("{} is used by {owner}.", shortcut.keycaps().join("")))
    }

    /// Why another app, or macOS, keeps `shortcut` from a binding recorded in Settings; none
    /// for a chord Dupin holds itself, which config.toml then says who has.
    pub fn used_elsewhere(&self, shortcut: &Shortcut) -> Option<String> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.ids.contains_key(shortcut) {
            return None;
        }
        drop(state);
        self.owner(shortcut)
            .map(|owner| format!("{} is used by {owner}.", shortcut.keycaps().join("")))
    }

    fn why(&self, shortcut: &Shortcut, error: &ShortcutError) -> String {
        match (error, self.panel.shortcut_owner(shortcut)) {
            (ShortcutError::Conflict, Some(owner)) => format!("{owner} already uses it"),
            (ShortcutError::Conflict, None) => "another app already uses it".to_owned(),
            (e, _) => e.to_string(),
        }
    }
}

fn refresh_system_conflict(
    failure: &mut Option<LaunchFailure>,
    installed: Option<&Shortcut>,
    owner_at: impl FnOnce(&Shortcut) -> Option<&'static str>,
) -> bool {
    let Some(LaunchFailure {
        shortcut: Some(shortcut),
        problem: LaunchProblem::Occupied(owner),
    }) = failure
    else {
        return false;
    };
    if Some(&*shortcut) != installed {
        return false;
    }
    if let Some(current_owner) = owner_at(shortcut) {
        *owner = current_owner.to_owned();
        false
    } else {
        *failure = None;
        true
    }
}

fn replacement_targets(
    state: &State,
    kind: LauncherShortcut,
    next: Option<&Shortcut>,
    system_owns: impl FnOnce(&Shortcut) -> bool,
) -> Result<HashMap<Shortcut, Target>, ShortcutError> {
    let matches_kind = |target: &Target| matches!(target, Target::Launcher(found) if *found == kind);
    if state.launch_failures[kind as usize].is_some()
        && let Some(shortcut) = next
        && state.targets.get(shortcut).is_some_and(matches_kind)
        && system_owns(shortcut)
    {
        return Err(ShortcutError::Conflict);
    }
    let mut targets = state.targets.clone();
    targets.retain(|_, target| !matches_kind(target));
    if let Some(next) = next
        && targets.insert(next.clone(), Target::Launcher(kind)).is_some()
    {
        return Err(ShortcutError::Conflict);
    }
    Ok(targets)
}

pub fn binding_targets(config: &Config) -> HashMap<Shortcut, &HotkeyTarget> {
    config
        .bindings
        .iter()
        .filter_map(|(accelerator, target)| Some((accelerator.parse().ok()?, target)))
        .collect()
}

/// Gives each command in `registry` that a `target = "dupin"` binding without a query opens
/// that binding's shortcut, for the rows naming the command to show.
pub fn mark_shortcuts(config: &Config, registry: &mut dupin_core::CommandRegistry) {
    let registered = registry.ids();
    for (accelerator, target) in &config.bindings {
        let HotkeyTarget::Dupin { command, query: None } = target else {
            continue;
        };
        let Some(command) = command.command() else {
            continue;
        };
        let (Ok(shortcut), Ok(command)) = (
            accelerator.parse::<Shortcut>(),
            crate::features::aliases::command(command, &registered),
        ) else {
            continue;
        };
        _ = registry.set_shortcut(&command, shortcut.keycaps().join(""));
    }
}

/// What in `config` already uses `shortcut`, in words, leaving out the binding written
/// `except`.
pub fn holder(config: &Config, shortcut: &Shortcut, except: Option<&str>) -> Option<String> {
    if let Some(kind) = LauncherShortcut::ALL
        .into_iter()
        .find(|kind| parse(kind.accelerator(config)).ok().flatten().as_ref() == Some(shortcut))
    {
        return Some(format!("the {} shortcut", kind.name()));
    }
    if let Some(key) = crate::list_keys::holding(config, shortcut) {
        return Some(format!("the {}", key.name()));
    }
    let (_, target) = config.bindings.iter().find(|(accelerator, _)| {
        Some(accelerator.as_str()) != except
            && accelerator.parse::<Shortcut>().ok().as_ref() == Some(shortcut)
    })?;
    Some(match target {
        HotkeyTarget::App { bundle_id, .. } => format!("the binding for {bundle_id}"),
        HotkeyTarget::Command { .. } => "a command binding".to_owned(),
        HotkeyTarget::Script { script, .. } => format!("the binding for {script}"),
        HotkeyTarget::Dupin { command, .. } => format!("the binding for {command}"),
    })
}

fn unchanged_binding(
    shortcut: &Shortcut,
    targets: &HashMap<Shortcut, Target>,
    previous: &HashMap<Shortcut, &HotkeyTarget>,
) -> bool {
    matches!(targets.get(shortcut), Some(Target::Configured(target)) if previous.get(shortcut) == Some(&target))
}

fn configured_targets(config: &Config) -> Result<HashMap<Shortcut, Target>, String> {
    let mut targets = HashMap::new();
    for kind in LauncherShortcut::ALL {
        let accelerator = kind.accelerator(config);
        if let Some(shortcut) =
            parse(accelerator).map_err(|_| format!("The {} shortcut is invalid.", kind.name()))?
        {
            insert_target(&mut targets, shortcut, Target::Launcher(kind))?;
        }
    }
    for (accelerator, target) in &config.bindings {
        let shortcut = accelerator
            .parse::<Shortcut>()
            .map_err(|_| "A shortcut in [bindings] is invalid.")?;
        let problem = match target {
            HotkeyTarget::App { bundle_id, .. }
                if bundle_id.is_empty() || bundle_id.contains(char::is_whitespace) =>
            {
                Some("an app bundle identifier")
            }
            HotkeyTarget::Command { command } if command.trim().is_empty() => Some("a shell command"),
            HotkeyTarget::Dupin {
                command,
                query: Some(_),
            } if !command.takes_query() => Some("a command or a web search to type its query into"),
            HotkeyTarget::App { .. }
            | HotkeyTarget::Command { .. }
            | HotkeyTarget::Script { .. }
            | HotkeyTarget::Dupin { .. } => None,
        };
        if let Some(required) = problem {
            return Err(format!("{} needs {required}.", shortcut.keycaps().join("")));
        }
        insert_target(&mut targets, shortcut, Target::Configured(target.clone()))?;
    }
    Ok(targets)
}

fn insert_target(
    targets: &mut HashMap<Shortcut, Target>,
    shortcut: Shortcut,
    target: Target,
) -> Result<(), String> {
    match targets.entry(shortcut) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(target);
            Ok(())
        }
        std::collections::hash_map::Entry::Occupied(entry) => Err(format!(
            "{} is assigned more than once.",
            entry.key().keycaps().join("")
        )),
    }
}

/// Whether what a `target = "dupin"` binding addresses is there now; `Err(None)` while the
/// extensions that may answer it are still loading.
pub type Commands<'a> = &'a dyn Fn(&Address) -> Result<(), Option<String>>;

/// What the bindings whose script or command is missing need, which leaves them out until it
/// appears. Without a catalogue, or while a command may still load, they are left out quietly.
fn filter_unavailable(
    targets: &mut HashMap<Shortcut, Target>,
    scripts: Option<&Catalog>,
    commands: Commands,
) -> Vec<String> {
    let mut problems = Vec::new();
    targets.retain(|shortcut, target| {
        let problem = match target {
            Target::Configured(HotkeyTarget::Script { script, args }) => {
                let Some(scripts) = scripts else {
                    return false;
                };
                match scripts.bound_script(script, args) {
                    Ok(_) => return true,
                    Err(problem) => format!(
                        "Couldn't register the {} binding for {script:?}. {problem}.",
                        shortcut.keycaps().join("")
                    ),
                }
            }
            Target::Configured(HotkeyTarget::Dupin { command, .. }) => match commands(command) {
                Ok(()) => return true,
                Err(None) => return false,
                Err(Some(problem)) => format!(
                    "Couldn't register the {} binding: it {problem}.",
                    shortcut.keycaps().join("")
                ),
            },
            _ => return true,
        };
        problems.push(problem);
        false
    });
    problems
}

fn sync<Id: Copy>(
    ids: &mut HashMap<Shortcut, Id>,
    targets: &HashMap<Shortcut, Target>,
    register: impl Fn(&Shortcut) -> Result<Id, ShortcutError>,
    unregister: impl Fn(Id) -> Result<(), ShortcutError>,
    mut skip: impl FnMut(&Shortcut, &ShortcutError) -> bool,
) -> Result<(), (Shortcut, ShortcutError)> {
    // Keep current registrations until all new chords have been acquired. A chord that
    // changes target keeps its registration, so two bindings can swap on reload.
    for shortcut in targets.keys() {
        if let std::collections::hash_map::Entry::Vacant(slot) = ids.entry(shortcut.clone()) {
            match register(shortcut) {
                Ok(id) => {
                    slot.insert(id);
                }
                Err(error) if skip(shortcut, &error) => {}
                Err(error) => return Err((shortcut.clone(), error)),
            }
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
    sync_transaction(ids, previous, targets, register, unregister, |_, _| false)
}

fn sync_transaction<Id: Copy>(
    ids: &mut HashMap<Shortcut, Id>,
    previous: &HashMap<Shortcut, Target>,
    targets: &HashMap<Shortcut, Target>,
    register: impl Fn(&Shortcut) -> Result<Id, ShortcutError>,
    unregister: impl Fn(Id) -> Result<(), ShortcutError>,
    skip: impl FnMut(&Shortcut, &ShortcutError) -> bool,
) -> Result<(), (Shortcut, ShortcutError)> {
    if let Err(error) = sync(ids, targets, &register, &unregister, skip) {
        if let Err((shortcut, error)) = sync(ids, previous, register, unregister, |_, _| false) {
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

    #[test]
    fn releasing_a_system_reservation_clears_the_warning_for_an_installed_chord() {
        let shortcut: rune::Shortcut = "cmd+space".parse().unwrap();
        let mut failure = Some(LaunchFailure {
            shortcut: Some(shortcut.clone()),
            problem: LaunchProblem::Occupied("Spotlight".into()),
        });

        assert!(!refresh_system_conflict(&mut failure, None, |_| panic!(
            "no registration"
        )));
        assert!(!refresh_system_conflict(
            &mut failure,
            Some(&shortcut),
            |installed| {
                assert_eq!(installed, &shortcut);
                Some("macOS")
            }
        ));
        assert!(
            matches!(&failure, Some(LaunchFailure { problem: LaunchProblem::Occupied(owner), .. }) if owner == "macOS")
        );
        assert!(refresh_system_conflict(&mut failure, Some(&shortcut), |_| None));
        assert!(failure.is_none());
    }

    fn app() -> HotkeyTarget {
        HotkeyTarget::App {
            bundle_id: "com.apple.calculator".into(),
            hide_if_frontmost: true,
        }
    }

    fn dupin(address: &str, query: Option<&str>) -> HotkeyTarget {
        HotkeyTarget::Dupin {
            command: Address::parse(address).unwrap(),
            query: query.map(str::to_owned),
        }
    }

    #[test]
    fn a_query_goes_only_with_a_command_or_a_web_search() {
        for (address, takes) in [
            ("clipboard", true),
            ("web:g", true),
            ("system/sleep", false),
            ("running-apps/com.apple.Safari", false),
            ("app:com.apple.calculator", false),
            ("file:~/Downloads", false),
        ] {
            let mut config = Config::default();
            config
                .bindings
                .insert("alt+cmd+q".into(), dupin(address, Some("x")));
            match configured_targets(&config) {
                Ok(_) => assert!(takes, "{address}"),
                Err(problem) => {
                    assert!(!takes, "{address}");
                    assert_eq!(
                        problem,
                        "⌥⌘Q needs a command or a web search to type its query into."
                    );
                }
            }
            config.bindings.insert("alt+cmd+q".into(), dupin(address, None));
            assert!(configured_targets(&config).is_ok(), "{address}");
        }
    }

    #[test]
    fn a_binding_to_a_row_is_skipped_with_the_reason_it_cant_run() {
        let mut config = Config::default();
        config
            .bindings
            .insert("alt+cmd+s".into(), dupin("system/shut-down", None));
        config
            .bindings
            .insert("alt+cmd+l".into(), dupin("system/sleep", None));
        let mut targets = configured_targets(&config).unwrap();
        let automation_off = |address: &Address| match address {
            Address::Row { command, row } if command == "system" => {
                crate::features::system::unavailable(row, false, true)
                    .map_or(Ok(()), |problem| Err(Some(problem)))
            }
            _ => Ok(()),
        };
        assert_eq!(
            filter_unavailable(&mut targets, None, &automation_off),
            [
                "Couldn't register the ⌥⌘S binding: it names `system/shut-down`, which runs only with System Command Automation on."
            ]
        );
        assert!(targets.contains_key(&"alt+cmd+l".parse().unwrap()));
        assert_eq!(
            holder(&config, &"alt+cmd+l".parse().unwrap(), None).as_deref(),
            Some("the binding for system/sleep")
        );
    }

    #[test]
    fn rejects_aliases_of_the_panel_or_another_binding() {
        let mut config = Config::default();
        config.bindings.insert("Command+Space".into(), app());
        assert!(configured_targets(&config).is_err());
        config.hotkey.clear();
        config.bindings.insert(
            "cmd+space".into(),
            HotkeyTarget::Command {
                command: "true".into(),
            },
        );
        assert!(configured_targets(&config).is_err());
    }

    #[test]
    fn a_command_binding_needs_a_non_blank_command() {
        let mut config = Config::default();
        for command in ["", " \t\n", "true"] {
            let target = HotkeyTarget::Command {
                command: command.into(),
            };
            config.bindings.insert("alt+cmd+g".into(), target.clone());
            if command.trim().is_empty() {
                assert_eq!(
                    configured_targets(&config).err().unwrap(),
                    "⌥⌘G needs a shell command."
                );
            } else {
                let targets = configured_targets(&config).unwrap();
                let shortcut = "alt+cmd+g".parse().unwrap();
                assert!(matches!(&targets[&shortcut], Target::Configured(actual) if actual == &target));
            }
        }
    }

    #[test]
    fn script_binding_is_skipped_until_its_catalogue_entry_is_available() {
        let mut config = Config::default();
        config.bindings.insert("ctrl+alt+c".into(), app());
        config.bindings.insert(
            "alt+cmd+t".into(),
            HotkeyTarget::Script {
                script: "fixture.sh".into(),
                args: Vec::new(),
            },
        );
        let shortcut: Shortcut = "alt+cmd+t".parse().unwrap();
        let configured = configured_targets(&config).unwrap();
        let mut targets = configured.clone();

        assert!(filter_unavailable(&mut targets, None, &|_| Ok(())).is_empty());
        assert!(!targets.contains_key(&shortcut));

        let mut catalog = Catalog::default();
        let mut targets = configured.clone();
        let problems = filter_unavailable(&mut targets, Some(&catalog), &|_| Ok(()));
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("⌥⌘T") && problems[0].contains("fixture.sh"));
        assert!(!targets.contains_key(&shortcut));
        assert_eq!(targets.len(), 2);

        let path = std::path::PathBuf::from("scripts/fixture.sh");
        let (header, _) = dupin_core::script::Header::parse(b"#!/bin/sh\n", "fixture").unwrap();
        catalog
            .scripts
            .insert(path.clone(), crate::scripts::catalog::Script { path, header });
        let mut targets = configured;
        assert!(filter_unavailable(&mut targets, Some(&catalog), &|_| Ok(())).is_empty());
        assert!(targets.contains_key(&shortcut));
        assert_eq!(targets.len(), 3);
    }

    #[test]
    fn a_dupin_binding_waits_for_its_command_and_names_one_that_is_missing() {
        let mut config = Config::default();
        config.bindings.insert("ctrl+alt+c".into(), app());
        for (chord, command) in [("cmd+shift+v", "clipboard"), ("alt+cmd+w", "word-count")] {
            config.bindings.insert(chord.into(), dupin(command, None));
        }
        let configured = configured_targets(&config).unwrap();
        let clipboard: Shortcut = "cmd+shift+v".parse().unwrap();
        let words: Shortcut = "alt+cmd+w".parse().unwrap();

        let mut targets = configured.clone();
        let loading = |address: &Address| {
            if address.command() == Some("clipboard") {
                Ok(())
            } else {
                Err(None)
            }
        };
        assert!(filter_unavailable(&mut targets, None, &loading).is_empty());
        assert!(targets.contains_key(&clipboard) && !targets.contains_key(&words));

        let mut targets = configured;
        let off = |address: &Address| Err(Some(format!("names `{address}`, which is off")));
        let problems = filter_unavailable(&mut targets, None, &off);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(
            problems
                .iter()
                .any(|problem| problem
                    == "Couldn't register the ⇧⌘V binding: it names `clipboard`, which is off."),
            "{problems:?}"
        );
        assert_eq!(targets.len(), 2, "the panel and the app binding stay");
    }

    #[test]
    fn a_dupin_binding_clashes_like_any_other() {
        let mut config = Config::default();
        config
            .bindings
            .insert("Command+Space".into(), dupin("clipboard", None));
        assert_eq!(
            configured_targets(&config).err().unwrap(),
            "⌘Space is assigned more than once."
        );
        let shortcut: Shortcut = "cmd+space".parse().unwrap();
        assert_eq!(
            holder(&config, &shortcut, None).as_deref(),
            Some("the panel shortcut")
        );
    }

    #[test]
    fn selection_shares_the_panel_and_app_binding_namespace() {
        let mut config = Config {
            selection_hotkey: "Command+Space".into(),
            ..Config::default()
        };
        assert!(configured_targets(&config).is_err());

        config.selection_hotkey = "ctrl+alt+c".into();
        let selection = config.selection_hotkey.parse::<Shortcut>().unwrap();
        assert!(matches!(
            configured_targets(&config).unwrap().get(&selection),
            Some(Target::Launcher(LauncherShortcut::Selection))
        ));
        config.bindings.insert("Control+Option+C".into(), app());
        assert!(configured_targets(&config).is_err());
    }

    #[test]
    fn rerecording_an_installed_launcher_only_refuses_a_chord_macos_still_uses() {
        let kind = LauncherShortcut::Panel;
        let shortcut: Shortcut = "cmd+space".parse().unwrap();
        let state = State {
            targets: HashMap::from([(shortcut.clone(), Target::Launcher(kind))]),
            launch_failures: [
                Some(LaunchFailure {
                    shortcut: Some(shortcut.clone()),
                    problem: LaunchProblem::Occupied("Spotlight".into()),
                }),
                None,
            ],
            ..State::default()
        };
        assert!(matches!(
            replacement_targets(&state, kind, Some(&shortcut), |_| true),
            Err(ShortcutError::Conflict)
        ));
        let targets = replacement_targets(&state, kind, Some(&shortcut), |_| false).unwrap();
        assert!(matches!(
            targets.get(&shortcut),
            Some(Target::Launcher(LauncherShortcut::Panel))
        ));
    }

    #[test]
    fn can_reassign_an_installed_chord_and_release_a_removed_chord() {
        let panel: Shortcut = "cmd+space".parse().unwrap();
        let app_key: Shortcut = "ctrl+alt+c".parse().unwrap();
        let mut ids = HashMap::from([(panel.clone(), 1), (app_key.clone(), 2)]);
        let previous = HashMap::from([
            (panel.clone(), Target::Launcher(LauncherShortcut::Panel)),
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
        let config = Config::default();
        let previous = configured_targets(&config).unwrap();
        let panel = config.hotkey.parse::<Shortcut>().unwrap();
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
    fn an_unavailable_unchanged_binding_leaves_other_shortcuts_active() {
        let mut previous = Config::default();
        previous.bindings.insert("Control+Option+C".into(), app());
        let blocked: Shortcut = "ctrl+alt+c".parse().unwrap();
        let available: Shortcut = "ctrl+alt+d".parse().unwrap();
        let panel = previous.hotkey.parse::<Shortcut>().unwrap();
        let active = configured_targets(&Config::default()).unwrap();
        let mut config = previous.clone();
        config.bindings.remove("Control+Option+C");
        config.bindings.insert("ctrl+alt+c".into(), app());
        config.bindings.insert("ctrl+alt+d".into(), app());
        let targets = configured_targets(&config).unwrap();
        let previous_bindings = binding_targets(&previous);
        let mut ids = HashMap::from([(panel.clone(), 1)]);

        sync_transaction(
            &mut ids,
            &active,
            &targets,
            |shortcut| {
                if shortcut == &blocked {
                    Err(ShortcutError::Conflict)
                } else {
                    Ok(2)
                }
            },
            |_| panic!("keep the active panel shortcut"),
            |shortcut, error| {
                *error == ShortcutError::Conflict && unchanged_binding(shortcut, &targets, &previous_bindings)
            },
        )
        .unwrap();
        assert_eq!(ids, HashMap::from([(panel, 1), (available, 2)]));
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
