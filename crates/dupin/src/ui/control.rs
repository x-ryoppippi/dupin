use std::sync::atomic::Ordering;

use dupin_core::Intent;
use rune::{Key, KeyEvent, TextEdit, Tx};

use super::Ui;

pub fn pick_indices(values: &[serde_json::Value], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            let label = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            label.to_lowercase().contains(&query).then_some(index)
        })
        .collect()
}

/// How long Enter stays inert after a caller shows the panel, so a key the user meant for
/// another app can't choose a result someone else put in front of them.
const CALLER_HOLD: f64 = 0.5;

#[derive(Default)]
pub(super) struct CallerHold(Option<f64>);

impl CallerHold {
    fn start(&mut self, now: f64) {
        self.0 = Some(now);
    }

    pub(super) fn holds(&self, now: f64) -> bool {
        self.0
            .is_some_and(|shown| (shown..shown + CALLER_HOLD).contains(&now))
    }
}

pub struct Picker {
    pub values: Vec<serde_json::Value>,
    pub selected: usize,
    pub(super) original: TextEdit,
    original_shown: bool,
    call: crate::control::Call,
}

impl Ui {
    pub fn control(&mut self, call: crate::control::Call, tx: &mut Tx) {
        use crate::control::Command;
        if call.cancelled.load(Ordering::Relaxed) {
            return;
        }
        match &call.request.command {
            Command::SnapshotsOpen { .. } if self.picker.is_some() => {
                call.finish(Err("Busy: picker active".into()));
            }
            Command::SnapshotsList {} | Command::SnapshotsOpen { .. } => {
                self.snapshots.update_if(move |s, _| {
                    s.control(call);
                    false
                });
            }
            Command::SnapshotsClose {} => {
                self.snapshots_after_close = false;
                self.snapshots.update(move |s, _| s.control_close(call));
            }
            Command::Open { query } => {
                if self.picker.is_some() {
                    call.finish(Err("Busy: picker active".into()));
                    return;
                }
                if let Some(query) = query {
                    if self.session.renaming().is_some() || self.session.destination().is_some() {
                        let effects = self.session.intent(Intent::Dismiss).unwrap_or_default();
                        self.apply(effects, tx);
                    }
                    self.query = TextEdit::new(query);
                    self.edited(tx);
                }
                self.show(tx);
                self.caller_hold.start(tx.clock());
                call.finish(Ok(serde_json::Value::Null));
            }
            Command::Pick { values } => {
                if self.picker.is_some() {
                    call.finish(Err("Busy: picker active".into()));
                    return;
                }
                let values = values.clone();
                let original = std::mem::replace(&mut self.query, TextEdit::new(""));
                self.action_screen = None;
                tx.wake_at(tx.clock() + 0.05);
                self.picker = Some(Picker {
                    values,
                    selected: 0,
                    original,
                    original_shown: self.shown,
                    call,
                });
                self.show(tx);
            }
            Command::ExtensionDataDelete { package } => {
                let bridge = self.extensions.bridge.clone();
                let package = dupin_extension_bridge::PackageId::new(package.as_str());
                std::thread::spawn(move || {
                    let result = package.map_err(|error| error.to_string()).and_then(|id| {
                        bridge
                            .delete_data(&id, dupin_extension_bridge::Origin::User)
                            .map_err(|failure| failure.to_string())
                    });
                    call.finish(result.map(|()| serde_json::Value::Null));
                });
            }
            Command::Sync { action } => {
                let action = action.clone();
                self.sync.control(action, call);
            }
            Command::ConfigOverlay {} => {
                std::thread::spawn(move || {
                    let source = match crate::settings::overlay::shared() {
                        Some(overlay) => overlay.read().map_err(|error| error.to_string()),
                        None => Ok(None),
                    };
                    call.finish(source.map(serde_json::Value::from));
                });
            }
            Command::ExtensionLockGet {} | Command::ExtensionLockSet { .. } => {
                std::thread::spawn(move || {
                    let result = crate::extensions::installed::answer(
                        crate::extensions::installed::shared(),
                        &call.request.command,
                    );
                    call.finish(result);
                });
            }
            Command::ExtensionGrantsForget { .. } | Command::ExtensionGrantsList { .. } => {
                let bridge = self.extensions.bridge.clone();
                std::thread::spawn(move || {
                    let result = crate::extensions::grants::answer(&bridge, &call.request.command);
                    call.finish(result);
                });
            }
            Command::ConfigGet {} => call.finish(
                serde_json::to_value(&*self.configuration.values.borrow())
                    .map_err(|_| "could not encode settings".into()),
            ),
            Command::ConfigSet { base, desired } => {
                if self.control_save.is_some() {
                    call.finish(Err("Busy: settings save pending".into()));
                    return;
                }
                if let Err(error) =
                    crate::settings::check_locked(base, desired, &self.configuration.locked_keys)
                {
                    call.finish(Err(error));
                    return;
                }
                let commands = self.bound_commands();
                let previous = self.configuration.values.borrow();
                {
                    let scripts = self.scripts.read().unwrap();
                    if let Err(error) = self.configuration.hotkey.reserve_for_save(
                        desired,
                        &previous,
                        self.scripts_catalog_ready.then_some(&*scripts),
                        &commands,
                    ) {
                        call.finish(Err(error));
                        return;
                    }
                }
                match self.configuration.files.save(base, desired) {
                    Ok(revision) => self.control_save = Some((revision, call)),
                    Err(error) => {
                        self.configuration.hotkey.discard_reservations();
                        call.finish(Err(error));
                    }
                }
            }
        }
    }

    pub(super) fn finish_pick(&mut self, value: Option<serde_json::Value>, tx: &mut Tx) {
        if let Some(call) = self.restore_picker() {
            if !self.shown {
                self.hide(tx);
            }
            tx.instant();
            call.finish(value.ok_or_else(|| "Cancelled".into()));
        }
    }

    pub(super) fn restore_picker(&mut self) -> Option<crate::control::Call> {
        let picker = self.picker.take()?;
        self.query = picker.original;
        self.shown = picker.original_shown;
        Some(picker.call)
    }

    pub(super) fn picker_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        match ev.key {
            Key::Escape => self.finish_pick(None, tx),
            Key::Enter => {
                let value = self
                    .picker
                    .as_ref()
                    .and_then(|p| {
                        pick_indices(&p.values, &self.query.text)
                            .get(p.selected)
                            .and_then(|index| p.values.get(*index))
                    })
                    .cloned();
                self.finish_pick(value, tx);
            }
            Key::Up => {
                let p = self.picker.as_mut().unwrap();
                p.selected = p.selected.saturating_sub(1);
            }
            Key::Down => {
                let p = self.picker.as_mut().unwrap();
                p.selected =
                    (p.selected + 1).min(pick_indices(&p.values, &self.query.text).len().saturating_sub(1));
            }
            _ => return false,
        }
        tx.instant();
        true
    }

    pub(super) fn poll_picker(&mut self, clock: f64, tx: &mut Tx) {
        if self
            .picker
            .as_ref()
            .is_some_and(|p| p.call.cancelled.load(Ordering::Relaxed))
        {
            self.finish_pick(None, tx);
        }
        if self.picker.is_some() {
            tx.wake_at(clock + 0.05);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_stays_inert_just_after_a_caller_shows_the_panel() {
        let mut hold = CallerHold::default();
        assert!(!hold.holds(10.0));

        hold.start(10.0);
        assert!(hold.holds(10.0));
        assert!(hold.holds(10.0 + CALLER_HOLD / 2.0));
        assert!(!hold.holds(10.0 + CALLER_HOLD));

        hold.start(10.3);
        assert!(hold.holds(10.0 + CALLER_HOLD));
        assert!(!hold.holds(10.3 + CALLER_HOLD));
    }
}
