use std::sync::Arc;

use dupin_core::{
    Batch, CommandError, CommandSpec, Config, Item, Payload, Provider, ProviderError, QueryEligibility,
    Request, View,
};
use dupin_mac::privacy::Permission;

use super::extensions::Port;
use super::{Bundled, Capability, Feature, PermissionUse};
use crate::clipboard::PACKAGE;
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;

pub(crate) const CLIPBOARD: &str = "clipboard.history";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    reload,
    permissions: &[PermissionUse {
        capability: Capability::Permission(Permission::Pasteboard),
        label: "clipboard history",
        enabled: |config| config.clipboard.enabled,
        // `clip` offers the request itself, so turning the history on only checks access.
        request_on_enable: false,
    }],
    bundled: Some(Bundled {
        package: PACKAGE,
        command: CLIPBOARD,
        enabled: |config| config.clipboard.enabled,
    }),
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    if config.clipboard.enabled {
        let clipboard = id(CLIPBOARD);
        let mut spec = CommandSpec::new(
            clipboard.clone(),
            config.clipboard.max_entries,
            presentation(
                "Search clipboard history",
                Some("Clipboard"),
                Some("Nothing copied yet  ·  ⌫ back to apps"),
                Some("No matches"),
                false,
                4.0,
            ),
        )
        .with_query_eligibility(QueryEligibility::Any);
        spec.private_results = true;
        commands.registry.register(spec)?;
        commands.registry.add_keyword(&clipboard, "clip")?;
    }
    Ok(())
}

fn start(providers: &mut Providers) {
    let port = providers.port(CLIPBOARD);
    let access = if rune::tooling::is_headless() {
        || dupin_mac::clipboard::Access::Allowed
    } else {
        dupin_mac::clipboard::access
    };
    providers.add(CLIPBOARD, Arc::new(ClipboardHistory { access, port }));
}

fn reload(ui: &mut Ui, config: &Config, _: &mut ConfiguredCommands) -> bool {
    let chosen = crate::extensions::preferences::choose(PACKAGE, preferences(&config.clipboard));
    let toggled = ui.apply_clipboard(&config.clipboard);
    if chosen || toggled {
        ui.preferences_changed(PACKAGE);
    }
    (chosen || toggled) && ui.session.command_id().as_str() == CLIPBOARD
}

/// The history's Settings rows and `[clipboard]` keys, as the bundled package's preferences.
fn preferences(config: &dupin_core::config::Clipboard) -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::from_iter([
        (
            "retention".into(),
            serde_json::to_value(config.retention).unwrap_or_default(),
        ),
        ("includeImages".into(), config.include_images.into()),
        ("includeFiles".into(), config.include_files.into()),
        ("maxEntries".into(), config.max_entries.into()),
    ])
}

/// Clipboard history: the bundled extension's catalogue, below a row that explains why nothing
/// is being recorded while macOS doesn't let Dupin read the clipboard in the background. That
/// access is Dupin's, not the extension's, so Dupin explains it and asks for it itself.
struct ClipboardHistory {
    access: fn() -> dupin_mac::clipboard::Access,
    port: Arc<Port>,
}

impl Provider for ClipboardHistory {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        self.search_batches(request, &mut |batch| emit(batch.items))
    }

    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
        if request.limit == 0 || request.is_cancelled() {
            return Ok(());
        }
        if let Some(row) = dupin_mac::clipboard::access_item((self.access)()) {
            emit(vec![row].into());
        }
        match self.port.provider() {
            Some(extension) => extension.search_batches(request, emit),
            None => {
                emit(vec![unavailable()].into());
                Ok(())
            }
        }
    }
}

fn unavailable() -> Item {
    Item {
        payload: Payload::default(),
        id: super::UNAVAILABLE_ROW.into(),
        view: View::Text {
            title: "Clipboard history isn't available".into(),
            subtitle: "Its extension is off or didn't load; Settings → Extensions says why.".into(),
        },
        record_usage: false,
        icon: None,
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: None,
        alternate: None,
        actions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_mac::clipboard::Access;

    struct Answers;

    impl Provider for Answers {
        fn search(&self, _: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
            let mut row = unavailable();
            row.view = View::Text {
                title: "The game is afoot.".into(),
                subtitle: "Just now · 18 characters".into(),
            };
            emit(vec![row]);
            Ok(())
        }
    }

    fn titles(history: &ClipboardHistory) -> Vec<String> {
        let mut rows = Vec::new();
        history
            .search(&Request::detached("", 30), &mut |batch| rows.extend(batch))
            .unwrap();
        rows.iter().map(|row| row.display_title().to_owned()).collect()
    }

    #[test]
    fn the_bundled_extension_answers_below_any_access_row() {
        let history = ClipboardHistory {
            access: || Access::Allowed,
            port: Arc::default(),
        };
        assert_eq!(titles(&history), ["Clipboard history isn't available"]);
        history.port.set(Some(Arc::new(Answers)));
        assert_eq!(titles(&history), ["The game is afoot."]);
        let unasked = ClipboardHistory {
            access: || Access::Unasked,
            port: history.port.clone(),
        };
        assert_eq!(
            titles(&unasked),
            [
                "Clipboard history needs permission to read the clipboard",
                "The game is afoot."
            ]
        );
    }

    #[test]
    fn settings_reach_the_package_as_its_preferences() {
        let config = Config::parse(
            "[clipboard]\nenabled = true\nretention = \"30d\"\ninclude_images = false\nmax_entries = 5000",
        )
        .unwrap();
        assert_eq!(
            serde_json::Value::Object(preferences(&config.clipboard)),
            serde_json::json!({
                "retention": "30d",
                "includeImages": false,
                "includeFiles": true,
                "maxEntries": 5000,
            })
        );
    }
}
