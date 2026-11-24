use std::sync::Arc;

use dupin_core::{CommandError, CommandSpec, Config};

use super::{Feature, default_query};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::extensions::preferences;
use crate::ui::{ActionStage, Ui};
use crate::web::{WEB_SEARCH, WebProvider, WebSearches};

pub(crate) const WEB_KEYWORDS: &str = "web.keywords";
const WEB_FALLBACKS: &str = "web.fallbacks";

pub(super) const FEATURE: Feature = Feature {
    configure,
    prepare,
    start,
    reload,
    ..Feature::EMPTY
};

fn configure(_: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let registry = &mut commands.registry;
    registry.register(CommandSpec::new(
        id(WEB_KEYWORDS),
        1,
        presentation("Web search", None, None, None, false, 0.0),
    ))?;
    registry.register(
        CommandSpec::new(
            id(WEB_FALLBACKS),
            20,
            presentation("Web search", None, None, None, false, 0.0),
        )
        .with_fallback(),
    )?;
    commands.defaults.extend([id(WEB_KEYWORDS), id(WEB_FALLBACKS)]);
    commands.pinned = Some(id(WEB_KEYWORDS));
    Ok(())
}

fn prepare(config: &Config, commands: &mut Configure) {
    let (web, notice) =
        crate::web::configured(config, |keyword| commands.registry.keyword(keyword).is_some());
    commands.web = web;
    commands.notices.extend(notice);
}

fn start(providers: &mut Providers) {
    let web = providers.registration.web.clone();
    providers.add(WEB_KEYWORDS, Arc::new(WebProvider::new(web.clone(), false)));
    providers.add(WEB_FALLBACKS, Arc::new(WebProvider::new(web, true)));
}

/// Runs after the extensions feature has registered this configuration's extensions, so their
/// keywords take precedence over the aliases other packages list and their catalogues are the
/// ones searched.
fn reload(ui: &mut Ui, _: &Config, commands: &mut ConfiguredCommands) -> bool {
    let configured = preferences::choose(WEB_SEARCH, std::mem::take(&mut commands.web.preferences));
    if configured {
        ui.preferences_changed(WEB_SEARCH);
    }
    let next = WebSearches::new(
        ui.web_search_sources(),
        commands.registry.keywords().map(str::to_owned).collect(),
    )
    .with_bridge(ui.extension_bridge());
    let changed = {
        let mut web = ui.web.write().unwrap();
        let changed = *web != next;
        *web = next;
        changed || configured
    };
    if changed
        && let Some(screen) = &mut ui.action_screen
        && matches!(screen.stage, ActionStage::Receivers { .. })
    {
        screen.stage = ActionStage::List;
    }
    changed && default_query(ui)
}
