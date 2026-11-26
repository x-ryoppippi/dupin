use dupin_core::{Backdrop, Config, DisplaySelection};
use rune::Tx;

use super::Ui;

impl Ui {
    pub fn apply_config(&mut self, config: &Config, tx: &mut Tx) {
        if self.theme.panel_width != config.theme.panel_width {
            tx.resize_window_instant((config.theme.panel_width as f32, crate::app::HEADER_HEIGHT));
        }
        self.theme = config.theme.clone();
        let previous_limit = self.session.command().result_limit;
        let mut registration =
            crate::commands::configured(config).expect("built-in command registration is valid");
        let recomposed = self.composition.apply(&config.advanced);
        if recomposed.kinds {
            self.extensions.forget_catalogues();
        }
        let feature_changed = crate::features::reload(self, config, &mut registration) || recomposed.any();
        // The extensions' commands joined the registry just now.
        crate::hotkey::mark_shortcuts(config, &mut registration.registry);
        let sources = registration.registry.ids();
        self.snapshots.update(move |s, _| s.set_sources(sources));
        let effects = self
            .session
            .set_commands(registration.registry)
            .expect("built-in command registration is valid");
        self.registration_notice = registration.notice;
        let bound = (
            config.bindings.clone(),
            self.session.command_ids(),
            self.extensions.settled(),
        );
        if self.bound_against.as_ref() != Some(&bound) {
            self.bound_against = Some(bound);
            // A pending `dupin config set` holds its new chords until its save lands.
            if self.control_save.is_none() {
                self.refresh_hotkeys();
            }
        }
        if !effects.is_empty() {
            self.apply(effects, tx);
        } else if feature_changed || self.session.command().result_limit != previous_limit {
            self.action_screen = None;
            let effects = self.session.refresh();
            self.apply(effects, tx);
        }
        self.position = config.position;
        self.display = config.display;
        if self.display != DisplaySelection::Active {
            self.display_notice = None;
        }
        self.input_source = config.input_source.clone();
        self.updater.set_enabled(config.auto_update);
        if let Some(telemetry) = crate::telemetry::get() {
            telemetry.set_enabled(config.telemetry);
        }
        self.sync.apply(&config.sync);
        if self.backdrop_pick != config.backdrop {
            self.backdrop = backdrop_source(&config.backdrop);
            self.backdrop_pick = config.backdrop.clone();
        }
    }

    pub fn reload_config(&mut self, update: crate::settings::Update, tx: &mut Tx) {
        if update.revision != self.configuration.files.revision() {
            return;
        }
        if let Some(locked_keys) = update.locked_keys {
            self.configuration.locked_keys = locked_keys;
        }
        let previous = self.configuration.values.borrow().clone();
        let (snapshot, read_problem) = reconcile_snapshot(
            update.result,
            &self.acknowledged_config,
            update.save_problem.is_some(),
        );
        let accepted = snapshot.and_then(|config| {
            self.apply_hotkeys(&config, &previous).map_err(|error| {
                format!("{error} Check the configuration files. Keeping the previous configuration.")
            })?;
            Ok(config)
        });
        let problem = match accepted {
            Ok(config) => {
                self.acknowledged_config = config.clone();
                if *self.configuration.values.borrow() != config {
                    *self.configuration.values.borrow_mut() = config.clone();
                    self.apply_config(&config, tx);
                }
                None
            }
            Err(error) => {
                if self.control_save.is_some() {
                    self.configuration.hotkey.discard_reservations();
                    self.refresh_hotkeys();
                }
                Some(error)
            }
        };
        let save_error = problem.clone().or_else(|| update.save_problem.clone());
        let problems: Vec<_> = [
            problem,
            read_problem,
            update.save_problem,
            update.watch_problem,
            update.save_notice,
        ]
        .into_iter()
        .flatten()
        .collect();
        let problem = (!problems.is_empty()).then(|| problems.join("; "));
        if self.config_problem != problem
            && let Some(problem) = &problem
        {
            crate::log::warn(problem);
        }
        self.config_problem = problem.clone();
        if self
            .control_save
            .as_ref()
            .is_some_and(|(revision, _)| *revision <= update.revision)
        {
            let (_, call) = self.control_save.take().unwrap();
            call.finish(match &save_error {
                Some(error) => Err(error.clone()),
                None => serde_json::to_value(&*self.configuration.values.borrow())
                    .map_err(|_| "could not encode settings".into()),
            });
        }
        let config = self.configuration.values.borrow().clone();
        let locked_keys = self.configuration.locked_keys.clone();
        self.prefs.update(move |prefs, _| {
            prefs.reflect_config(update.revision, config, locked_keys, problem);
        });
    }
}

/// The backdrop `pick` names, logging why it can't be used.
pub(super) fn backdrop_source(pick: &Backdrop) -> crate::shader::Source {
    let source = crate::shader::Source::new(pick);
    if let Some(problem) = source.problem() {
        crate::log::warn(problem);
    }
    source
}

fn reconcile_snapshot(
    snapshot: Result<Config, String>,
    acknowledged: &Config,
    save_failed: bool,
) -> (Result<Config, String>, Option<String>) {
    match snapshot {
        Err(error) if save_failed => (Ok(acknowledged.clone()), Some(error)),
        snapshot => (snapshot, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_core::{Effect, History, Session};

    #[test]
    fn cosmetic_configuration_retains_query_selection_history_and_pending_search() {
        let base = Config::default();
        let mut session = Session::new(
            History::default(),
            crate::commands::configured(&base).unwrap().registry,
        )
        .unwrap();
        let effects = session.set_query("example");
        let (origin, generation) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Search {
                    command, generation, ..
                } => Some((command.clone(), *generation)),
                _ => None,
            })
            .unwrap();
        let before = (
            session.query().to_owned(),
            session.selected(),
            session.history().clone(),
        );
        let mut themed = base;
        themed.theme.row_height = 100;
        themed.theme.light.accent = Some("#123456".into());
        session
            .set_commands(crate::commands::configured(&themed).unwrap().registry)
            .unwrap();
        assert_eq!(
            (
                session.query().to_owned(),
                session.selected(),
                session.history().clone()
            ),
            before
        );
        assert!(session.pending(&origin, generation));
        let invalid = Config::parse("[theme]\npanel_width = nan").map_err(|e| e.to_string());
        let (retained, problem) = reconcile_snapshot(invalid, &themed, true);
        assert_eq!(retained.unwrap(), themed);
        assert!(problem.is_some());
    }

    #[test]
    fn a_failed_save_and_invalid_snapshot_restore_acknowledged_values() {
        let acknowledged = Config::default();
        let (restored, problem) = reconcile_snapshot(Err("invalid TOML".into()), &acknowledged, true);
        assert_eq!(restored, Ok(acknowledged.clone()));
        assert_eq!(problem.as_deref(), Some("invalid TOML"));
        let disk = Config {
            file_limit: 30,
            ..acknowledged.clone()
        };
        assert_eq!(
            reconcile_snapshot(Ok(disk.clone()), &acknowledged, true),
            (Ok(disk), None)
        );
        assert!(
            reconcile_snapshot(Err("invalid TOML".into()), &acknowledged, false)
                .0
                .is_err()
        );
    }
}
