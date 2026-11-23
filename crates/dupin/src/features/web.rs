use std::sync::Arc;

use dupin_core::{CommandError, CommandSpec, Config};

use super::{Feature, default_query};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::{ActionStage, Ui};
use crate::web::{WebCatalog, WebProvider};

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
    let (catalog, notice) =
        WebCatalog::compile(config, |keyword| commands.registry.keyword(keyword).is_some());
    commands.web = catalog;
    commands.notices.extend(notice);
}

fn start(providers: &mut Providers) {
    let web = providers.registration.web.clone();
    providers.add(WEB_KEYWORDS, Arc::new(WebProvider::new(web.clone(), false)));
    providers.add(WEB_FALLBACKS, Arc::new(WebProvider::new(web, true)));
}

fn reload(ui: &mut Ui, _: &Config, commands: &mut ConfiguredCommands) -> bool {
    let changed = {
        let mut web = ui.web.write().unwrap();
        let next = std::mem::take(&mut commands.web);
        let changed = *web != next;
        *web = next;
        changed
    };
    if changed
        && let Some(screen) = &mut ui.action_screen
        && matches!(screen.stage, ActionStage::Receivers { .. })
    {
        screen.stage = ActionStage::List;
    }
    changed && default_query(ui)
}
