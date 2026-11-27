//! `[aliases]`: the user's own shorthands, registered after every built-in and configured
//! keyword and web search, and before any extension's keyword.

use std::path::{Path, PathBuf};

use dupin_core::config::AliasTarget;
use dupin_core::{CommandId, Config, KeywordHolder};

use super::Feature;
use crate::commands::{Configure, id};

pub(super) const FEATURE: Feature = Feature {
    prepare,
    ..Feature::EMPTY
};

/// The commands an alias names by these names, beside the packages that port them.
const COMMANDS: &[(&str, &str)] = &[
    ("files", super::files::FILES),
    ("contents", super::files::CONTENTS),
    ("calendar", super::calendar::CALENDAR),
    ("clipboard", super::clipboard::CLIPBOARD),
    ("dictionary", super::dictionary::DICTIONARY),
    ("snippets", super::snippets::SNIPPETS),
    ("snapshots", super::snapshots::SNAPSHOTS),
];

fn prepare(config: &Config, commands: &mut Configure) {
    let application = commands.application;
    register(config, commands, application);
}

/// The built-in command `name` names, whether or not its feature is on.
pub(crate) fn built_in(name: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, command)| *command)
        .or_else(|| super::ported_command(name))
}

/// The command `name` reaches among the `registered` command IDs, as an alias or a binding
/// names it: a built-in's name or an extension's package name; or why it reaches none.
pub(crate) fn command(name: &str, registered: &[String]) -> Result<CommandId, String> {
    let (command, off) = match built_in(name) {
        Some(command) => (command.to_owned(), format!("names `{name}`, which is off")),
        None => (
            format!("extension.{name}"),
            format!("names `{name}`, which is neither a command nor an extension that is on"),
        ),
    };
    if !registered.contains(&command) {
        return Err(off);
    }
    Ok(id(&command))
}

/// The name an alias or a binding gives the command `id`, for a built-in or an extension.
pub(crate) fn binding_name(id: &str) -> Option<&str> {
    COMMANDS
        .iter()
        .find(|(_, command)| *command == id)
        .map(|(name, _)| *name)
        .or_else(|| super::ported_package(id))
        .or_else(|| id.strip_prefix("extension."))
}

fn register(config: &Config, commands: &mut Configure, application: impl Fn(&str) -> Option<PathBuf>) {
    for (alias, target) in &config.aliases.0 {
        if let Err(problem) = add(commands, alias, target, &application) {
            commands.alias_problems.insert(alias.clone(), problem);
        }
    }
    if !commands.alias_problems.is_empty() {
        let problems: Vec<_> = commands
            .alias_problems
            .iter()
            .map(|(alias, problem)| format!("`{alias}` {problem}"))
            .collect();
        commands.notices.push(format!(
            "Some aliases in config.toml were left out: {}.",
            problems.join("; ")
        ));
    }
}

fn add(
    commands: &mut Configure,
    alias: &str,
    target: &AliasTarget,
    application: &impl Fn(&str) -> Option<PathBuf>,
) -> Result<(), String> {
    if let Some(holder) = commands.registry.holder(alias) {
        return Err(taken(commands, holder));
    }
    match target {
        AliasTarget::Command(name) => match built_in(name) {
            Some(command) => {
                let command = id(command);
                if !commands.registry.ids().iter().any(|id| id == command.as_str()) {
                    return Err(format!("names `{name}`, which is off"));
                }
                commands
                    .registry
                    .add_keyword(&command, alias)
                    .map_err(|error| error.to_string())
            }
            // An extension's command registers once the extensions load.
            None => {
                commands.package_aliases.push((alias.to_owned(), name.clone()));
                Ok(())
            }
        },
        AliasTarget::Web(search) => commands
            .registry
            .add_web_shorthand(alias, search)
            .map_err(|_| format!("names the web search `{search}`, which isn't one")),
        AliasTarget::App(bundle) => {
            let path = application(bundle)
                .ok_or_else(|| format!("names the app `{bundle}`, which isn't installed"))?;
            let title = title(&path);
            commands
                .registry
                .add_path_alias(alias, path, title)
                .map_err(|error| error.to_string())
        }
        AliasTarget::File(path) => {
            let path = expand(path);
            if !path.exists() {
                return Err(format!("names `{}`, which doesn't exist", path.display()));
            }
            let title = title(&path);
            commands
                .registry
                .add_path_alias(alias, path, title)
                .map_err(|error| error.to_string())
        }
    }
}

/// Why an alias can't take a keyword that `holder` already has.
fn taken(commands: &Configure, holder: KeywordHolder) -> String {
    match holder {
        KeywordHolder::Command(command) => format!("is already a keyword of {}", name(commands, command)),
        KeywordHolder::Web(title) => format!("is already the keyword of {title}"),
        KeywordHolder::Path(path) => format!("is already an alias for {}", path.display()),
    }
}

fn name(commands: &Configure, command: &CommandId) -> String {
    commands
        .registry
        .entries()
        .into_iter()
        .find(|entry| entry.target == dupin_core::KeywordTarget::Command(command.clone()))
        .map_or_else(|| format!("`{command}`"), |entry| entry.title)
}

fn title(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    name.strip_suffix(".app").map(str::to_owned).unwrap_or(name)
}

pub(crate) fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => Path::new(&home).join(rest),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use dupin_core::{History, Session};

    use super::*;
    use crate::commands::configured;

    fn config(aliases: &str) -> Config {
        Config::parse(&format!("[calendar]\nenabled = true\n[aliases]\n{aliases}")).unwrap()
    }

    #[test]
    fn aliases_reach_commands_web_searches_apps_and_files() {
        let folder = tempfile::tempdir().unwrap();
        let config = config(&format!(
            "c = \"calendar\"\nlookup = \"dictionary\"\ngoo = \"web:g\"\nff = \"app:org.mozilla.firefox\"\ntmp = \"file:{}\"\nwc = \"word-count\"",
            folder.path().display()
        ));
        let commands = crate::commands::configured_with(&config, |bundle| {
            (bundle == "org.mozilla.firefox").then(|| PathBuf::from("/Applications/Firefox.app"))
        })
        .unwrap();
        let registry = &commands.registry;
        assert_eq!(
            registry.keyword("c").map(CommandId::as_str),
            Some("calendar.events")
        );
        assert_eq!(
            registry.keyword("lookup").map(CommandId::as_str),
            Some("dictionary.lookup")
        );
        assert_eq!(registry.web_search("goo"), Some("g"));
        assert_eq!(
            registry.holder("ff"),
            Some(KeywordHolder::Path(Path::new("/Applications/Firefox.app")))
        );
        assert_eq!(registry.holder("tmp"), Some(KeywordHolder::Path(folder.path())));
        assert_eq!(
            commands.package_aliases,
            [("wc".to_owned(), "word-count".to_owned())]
        );
        assert!(
            commands.alias_problems.is_empty(),
            "{:?}",
            commands.alias_problems
        );
        // The registry lists the shorthand with the search it reaches.
        let google = registry
            .entries()
            .into_iter()
            .find(|entry| entry.title == "Search Google")
            .unwrap();
        assert_eq!(google.keywords, ["g", "goo"]);
    }

    #[test]
    fn built_in_and_configured_keywords_win_and_each_problem_is_reported() {
        let config = config(
            "d = \"calendar\"\ng = \"clipboard\"\nin = \"snapshots\"\nx = \"clipboard\"\ny = \"web:nope\"\nz = \"app:com.example.missing\"\nw = \"file:/nonexistent/dupin\"",
        );
        let configured = configured(&config).unwrap();
        let problems = &configured.alias_problems;
        assert_eq!(problems["d"], "is already a keyword of Dictionary");
        assert_eq!(problems["g"], "is already the keyword of Search Google");
        assert_eq!(problems["in"], "is already a keyword of File Contents");
        assert_eq!(problems["x"], "names `clipboard`, which is off");
        assert_eq!(problems["y"], "names the web search `nope`, which isn't one");
        assert_eq!(
            problems["z"],
            "names the app `com.example.missing`, which isn't installed"
        );
        assert!(problems["w"].contains("which doesn't exist"));
        let notice = configured.notice.unwrap();
        assert!(
            notice.contains("Some aliases in config.toml were left out: `d` is already"),
            "{notice}"
        );
        // The built-in keywords keep their commands.
        assert_eq!(
            configured.registry.keyword("d").map(CommandId::as_str),
            Some("dictionary.lookup")
        );
        assert_eq!(configured.registry.web_search("g"), Some("g"));
    }

    #[test]
    fn a_command_alias_enters_the_command_as_its_keyword_does() {
        use dupin_core::{Effect, Intent};

        let configured = configured(&config("c = \"calendar\"")).unwrap();
        let mut session = Session::new(History::default(), configured.registry).unwrap();
        session.set_query("c");
        let effects = session.intent(Intent::Space).unwrap();
        assert_eq!(effects[0], Effect::ClearQuery);
        assert_eq!(session.command_id().as_str(), "calendar.events");
    }

    #[test]
    fn per_feature_keywords_keep_working_beside_aliases() {
        let config = Config::parse(
            "dictionary_keywords = [\"word\"]\nsnippet_keywords = [\"sn\"]\n[aliases]\nlookup = \"dictionary\"\nsnips = \"snippets\"",
        )
        .unwrap();
        let configured = configured(&config).unwrap();
        let registry = &configured.registry;
        for (keyword, command) in [
            ("word", "dictionary.lookup"),
            ("lookup", "dictionary.lookup"),
            ("sn", "snippets.search"),
            ("snips", "snippets.search"),
        ] {
            assert_eq!(registry.keyword(keyword).map(CommandId::as_str), Some(command));
        }
        assert_eq!(
            registry.keyword("define"),
            None,
            "dictionary_keywords replaces the defaults"
        );
        let dictionary = registry
            .entries()
            .into_iter()
            .find(|entry| entry.title == "Dictionary")
            .unwrap();
        assert_eq!(dictionary.keywords, ["word", "lookup"]);
    }

    #[test]
    fn an_alias_for_an_extension_never_costs_it_its_own_keyword() {
        use std::sync::Arc;

        use crate::extensions::tests::extension;
        use crate::features::extensions::add_extensions;

        let config =
            Config::parse("[aliases]\nwc = \"word-count\"\nwords = \"word-count\"\nwc2 = \"other\"").unwrap();
        let mut commands = configured(&config).unwrap();
        let added = add_extensions(&mut commands, &[Arc::new(extension("Word Count", "wc"))]);
        let words = id("extension.word-count");
        assert_eq!(commands.registry.keyword("wc"), Some(&words));
        assert_eq!(commands.registry.keyword("words"), Some(&words));
        assert_eq!(added.problems.len(), 1, "{:?}", added.problems);
        assert!(
            added
                .problems
                .iter()
                .any(|problem| problem.contains("alias `wc2` names `other`"))
        );

        // Another package's keyword never takes an alias.
        let config = Config::parse("[aliases]\nwc = \"calendar\"\n[calendar]\nenabled = true").unwrap();
        let mut configured = configured(&config).unwrap();
        let added = add_extensions(&mut configured, &[Arc::new(extension("Word Count", "wc"))]);
        assert_eq!(
            configured.registry.keyword("wc").map(CommandId::as_str),
            Some("calendar.events")
        );
        assert_eq!(added.problems.len(), 1);
    }

    #[test]
    fn two_aliases_for_one_folder_list_it_once() {
        let folder = tempfile::tempdir().unwrap();
        let config = config(&format!(
            "doc = \"file:{0}\"\ndocs = \"file:{0}\"",
            folder.path().display()
        ));
        let configured = configured(&config).unwrap();
        let listed: Vec<_> = configured
            .registry
            .entries()
            .into_iter()
            .filter(|entry| entry.target == dupin_core::KeywordTarget::Path(folder.path().to_path_buf()))
            .collect();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].keywords, ["doc", "docs"]);
    }
}
