use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommandId(String);

impl CommandId {
    pub fn new(value: impl Into<String>) -> Result<Self, CommandError> {
        let value = value.into();
        let mut parts = value.split('.');
        let valid_part = |part: &str| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        };
        if parts.next().is_some_and(valid_part)
            && parts.next().is_some_and(valid_part)
            && parts.next().is_none()
        {
            Ok(Self(value))
        } else {
            Err(CommandError::InvalidId(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryPresentation {
    pub placeholder: String,
    pub chip: Option<String>,
    pub empty_hint: Option<String>,
    pub no_results_hint: Option<String>,
    pub show_searching: bool,
    pub backdrop: f32,
}

impl QueryPresentation {
    #[cfg(test)]
    pub(crate) fn new(placeholder: impl Into<String>) -> Self {
        Self {
            placeholder: placeholder.into(),
            chip: None,
            empty_hint: None,
            no_results_hint: None,
            show_searching: false,
            backdrop: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryEligibility {
    /// The command is eligible when the query contains non-whitespace text.
    #[default]
    NonEmpty,
    /// The command is eligible only when the query is empty or whitespace.
    EmptyOnly,
    Any,
}

impl QueryEligibility {
    pub(crate) fn allows(self, query: &str) -> bool {
        match self {
            Self::NonEmpty => !query.trim().is_empty(),
            Self::EmptyOnly => query.trim().is_empty(),
            Self::Any => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandSpec {
    id: CommandId,
    /// What lists of commands call it, where its chip is not a name of its own.
    title: Option<String>,
    /// What its chip and the rows that name it show beside its name.
    icon: Option<crate::Icon>,
    pub result_limit: usize,
    candidate_limit: Option<usize>,
    fallback: bool,
    pub private_results: bool,
    pub presentation: QueryPresentation,
    pub deadline: std::time::Duration,
    pub(crate) query_eligibility: QueryEligibility,
    /// Whether rows that name commands while the main search is typed, and `?`, name it.
    hinted: bool,
}

impl CommandSpec {
    pub fn new(id: CommandId, result_limit: usize, presentation: QueryPresentation) -> Self {
        Self {
            id,
            title: None,
            icon: None,
            result_limit,
            candidate_limit: None,
            fallback: false,
            private_results: false,
            presentation,
            deadline: std::time::Duration::from_secs(30),
            query_eligibility: QueryEligibility::NonEmpty,
            hinted: true,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_icon(mut self, icon: crate::Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn icon(&self) -> Option<&crate::Icon> {
        self.icon.as_ref()
    }

    /// The command's name: its title, or else its chip or placeholder.
    fn title(&self) -> &str {
        self.title
            .as_deref()
            .or(self.presentation.chip.as_deref())
            .unwrap_or(&self.presentation.placeholder)
    }

    /// Keeps the command out of the rows that name commands as the main search's query is
    /// typed, and out of `?`, while its keywords still enter it.
    pub fn unhinted(mut self) -> Self {
        self.hinted = false;
        self
    }

    pub fn hinted(&self) -> bool {
        self.hinted
    }

    pub fn with_query_eligibility(mut self, eligibility: QueryEligibility) -> Self {
        self.query_eligibility = eligibility;
        self
    }

    pub fn with_candidate_limit(mut self, limit: usize) -> Self {
        self.candidate_limit = Some(limit);
        self
    }

    /// Mark this provider's results as fallback results for the default search.
    pub fn with_fallback(mut self) -> Self {
        self.fallback = true;
        self
    }

    pub(crate) fn is_fallback(&self) -> bool {
        self.fallback
    }

    /// The total candidate budget, before ranking and the visible result limit.
    pub fn candidate_limit(&self) -> usize {
        self.candidate_limit
            .unwrap_or(self.result_limit)
            .max(self.result_limit)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    InvalidId(String),
    DuplicateId(CommandId),
    InvalidKeyword(String),
    UnknownCommand(CommandId),
    KeywordConflict { keyword: String, command: CommandId },
    WebAliasConflict(String),
    UnknownWebSearch(String),
    MissingDefault(CommandId),
    MissingSpaceTarget(CommandId),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(id) => write!(f, "command ID `{id}` must be plugin.command"),
            Self::DuplicateId(id) => write!(f, "command `{id}` is registered more than once"),
            Self::InvalidKeyword(keyword) => {
                write!(
                    f,
                    "keyword `{keyword}` must not be empty or have outer whitespace"
                )
            }
            Self::UnknownCommand(id) => write!(f, "command `{id}` is not registered"),
            Self::KeywordConflict { keyword, command } => write!(
                f,
                "keyword `{keyword}` for `{command}` is already assigned to another command"
            ),
            Self::WebAliasConflict(alias) => write!(f, "`{alias}` is already a keyword"),
            Self::UnknownWebSearch(search) => write!(f, "no web search answers to `{search}`"),
            Self::MissingDefault(id) => write!(f, "default command `{id}` is not registered"),
            Self::MissingSpaceTarget(id) => write!(f, "Space target `{id}` is not registered"),
        }
    }
}

impl std::error::Error for CommandError {}

/// What answers a keyword.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeywordHolder<'a> {
    Command(&'a CommandId),
    /// A web search, by its title.
    Web(&'a str),
    /// An app, file or folder an alias shows, by its path.
    Path(&'a Path),
}

/// What a keyword reaches: a command's own search, or a web search the main search answers.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeywordTarget {
    Command(CommandId),
    /// A web search, by its own alias.
    Web(String),
    /// An app, file or folder, which the row for the keyword opens.
    Path(PathBuf),
}

impl fmt::Display for KeywordTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(id) => write!(f, "command:{id}"),
            Self::Web(alias) => write!(f, "web:{alias}"),
            Self::Path(path) => write!(f, "path:{}", path.display()),
        }
    }
}

/// One command, or web search, and the keywords that reach it, shortest first. The files
/// search that Space on an empty query enters has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeywordEntry {
    pub target: KeywordTarget,
    pub title: String,
    pub subtitle: String,
    /// What the row naming it shows: the command's own icon, a globe for a web search, or the
    /// Finder icon of the app, file or folder.
    pub(crate) icon: Option<crate::Icon>,
    pub keywords: Vec<String>,
}

/// A web search's alias, or a shorthand for one.
#[derive(Clone, Debug)]
struct WebAlias {
    title: String,
    /// The alias the search itself answers to.
    search: String,
}

fn checked(keyword: String) -> Result<String, CommandError> {
    if keyword.trim().is_empty() || keyword.trim() != keyword {
        Err(CommandError::InvalidKeyword(keyword))
    } else {
        Ok(keyword)
    }
}

/// What a web search's rows show: its keyword's row and the row that searches for the query.
pub const WEB_SEARCH_SYMBOL: &str = "globe";

#[derive(Clone, Debug)]
pub struct CommandRegistry {
    default: CommandId,
    space_on_empty: Option<CommandId>,
    destination_search: Option<CommandId>,
    commands: HashMap<CommandId, CommandSpec>,
    keywords: HashMap<String, CommandId>,
    /// Web search aliases, and shorthands for them, by alias.
    web: BTreeMap<String, WebAlias>,
    /// Apps, files and folders aliases show, by alias, with their titles.
    paths: BTreeMap<String, (PathBuf, String)>,
    defaults: Vec<CommandId>,
    pinned: Option<CommandId>,
    browser: Option<CommandId>,
}

impl CommandRegistry {
    pub fn new(default: CommandId, space_on_empty: Option<CommandId>) -> Self {
        Self {
            defaults: vec![default.clone()],
            pinned: None,
            browser: None,
            default,
            space_on_empty,
            destination_search: None,
            commands: HashMap::new(),
            keywords: HashMap::new(),
            web: BTreeMap::new(),
            paths: BTreeMap::new(),
        }
    }

    /// Use this file-name search provider for the destination picker.
    pub fn set_destination_search(&mut self, command: CommandId) -> Result<(), CommandError> {
        if !self.contains(&command) {
            return Err(CommandError::UnknownCommand(command));
        }
        self.destination_search = Some(command);
        Ok(())
    }

    pub(crate) fn destination_search(&self) -> Option<&CommandId> {
        self.destination_search.as_ref()
    }

    pub fn set_default_providers(
        &mut self,
        providers: Vec<CommandId>,
        pinned: Option<CommandId>,
    ) -> Result<(), CommandError> {
        if providers.is_empty() {
            return Err(CommandError::MissingDefault(self.default.clone()));
        }
        for id in providers.iter().chain(pinned.iter()) {
            if !self.contains(id) || !providers.contains(id) {
                return Err(CommandError::UnknownCommand(id.clone()));
            }
        }
        let mut unique = std::collections::HashSet::new();
        self.defaults = providers
            .into_iter()
            .filter(|id| unique.insert(id.clone()))
            .collect();
        self.pinned = pinned;
        Ok(())
    }

    /// Add `command` to the default search right after `after`, or last when `after` isn't
    /// one of its providers.
    pub fn add_default_provider(
        &mut self,
        command: CommandId,
        after: &CommandId,
    ) -> Result<(), CommandError> {
        if !self.contains(&command) {
            return Err(CommandError::UnknownCommand(command));
        }
        if self.defaults.contains(&command) {
            return Ok(());
        }
        let at = self
            .defaults
            .iter()
            .position(|id| id == after)
            .map_or(self.defaults.len(), |index| index + 1);
        self.defaults.insert(at, command);
        Ok(())
    }

    /// The commands the default search runs, in order.
    pub fn default_providers(&self) -> &[CommandId] {
        &self.defaults
    }

    pub(crate) fn providers(&self, command: &CommandId) -> Vec<CommandId> {
        if command == &self.default {
            self.defaults.clone()
        } else {
            vec![command.clone()]
        }
    }

    pub(crate) fn pinned(&self) -> Option<&CommandId> {
        self.pinned.as_ref()
    }

    /// The command that lists a folder entered from a result.
    pub fn set_browser(&mut self, command: CommandId) -> Result<(), CommandError> {
        if !self.contains(&command) {
            return Err(CommandError::UnknownCommand(command));
        }
        self.browser = Some(command);
        Ok(())
    }

    pub(crate) fn browser(&self) -> Option<&CommandId> {
        self.browser.as_ref()
    }

    pub fn register(&mut self, command: CommandSpec) -> Result<(), CommandError> {
        if self.commands.contains_key(&command.id) {
            return Err(CommandError::DuplicateId(command.id));
        }
        self.commands.insert(command.id.clone(), command);
        Ok(())
    }

    /// Gives a registered command the icon its chip and the rows naming it show, as for a
    /// built-in command a bundled extension answers, whose package names the icon.
    pub fn set_icon(&mut self, command: &CommandId, icon: crate::Icon) -> Result<(), CommandError> {
        let spec = self
            .commands
            .get_mut(command)
            .ok_or_else(|| CommandError::UnknownCommand(command.clone()))?;
        spec.icon = Some(icon);
        Ok(())
    }

    pub fn add_keyword(
        &mut self,
        command: &CommandId,
        keyword: impl Into<String>,
    ) -> Result<(), CommandError> {
        if !self.commands.contains_key(command) {
            return Err(CommandError::UnknownCommand(command.clone()));
        }
        let keyword = checked(keyword.into())?;
        if self.web.contains_key(&keyword) || self.paths.contains_key(&keyword) {
            return Err(CommandError::KeywordConflict {
                keyword,
                command: command.clone(),
            });
        }
        if let Some(existing) = self.keywords.get(&keyword) {
            if existing != command {
                return Err(CommandError::KeywordConflict {
                    keyword,
                    command: command.clone(),
                });
            }
            return Ok(());
        }
        self.keywords.insert(keyword, command.clone());
        Ok(())
    }

    /// Reserve `alias` for the web search called `title`, which a row of the main search
    /// answers rather than a command.
    pub fn add_web_alias(
        &mut self,
        alias: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<(), CommandError> {
        let alias = checked(alias.into())?;
        self.reserve(&alias)?;
        let search = alias.clone();
        self.web.insert(
            alias,
            WebAlias {
                title: title.into(),
                search,
            },
        );
        Ok(())
    }

    /// Reserve `alias` for the web search `search` already answers to, so that typing it takes
    /// a query as `search` does.
    pub fn add_web_shorthand(&mut self, alias: impl Into<String>, search: &str) -> Result<(), CommandError> {
        let alias = checked(alias.into())?;
        let Some(target) = self.web.get(search).filter(|target| target.search == search) else {
            return Err(CommandError::UnknownWebSearch(search.to_owned()));
        };
        let title = target.title.clone();
        self.reserve(&alias)?;
        self.web.insert(
            alias,
            WebAlias {
                title,
                search: search.to_owned(),
            },
        );
        Ok(())
    }

    /// Reserve `alias` for the app, file or folder at `path`, called `title`.
    pub fn add_path_alias(
        &mut self,
        alias: impl Into<String>,
        path: PathBuf,
        title: impl Into<String>,
    ) -> Result<(), CommandError> {
        let alias = checked(alias.into())?;
        self.reserve(&alias)?;
        self.paths.insert(alias, (path, title.into()));
        Ok(())
    }

    fn reserve(&self, alias: &str) -> Result<(), CommandError> {
        if self.holder(alias).is_some() {
            return Err(CommandError::WebAliasConflict(alias.to_owned()));
        }
        Ok(())
    }

    /// What answers `keyword`: a command, a web search, or an app, file or folder.
    pub fn holder(&self, keyword: &str) -> Option<KeywordHolder<'_>> {
        self.keywords
            .get(keyword)
            .map(KeywordHolder::Command)
            .or_else(|| self.web.get(keyword).map(|web| KeywordHolder::Web(&web.title)))
            .or_else(|| self.paths.get(keyword).map(|(path, _)| KeywordHolder::Path(path)))
    }

    /// The web search `alias` reaches: its own alias, for a shorthand.
    pub fn web_search(&self, alias: &str) -> Option<&str> {
        self.web.get(alias).map(|web| web.search.as_str())
    }

    /// Every web search shorthand, with the alias of the search it reaches.
    pub fn web_shorthands(&self) -> impl Iterator<Item = (&str, &str)> {
        self.web
            .iter()
            .filter(|(alias, web)| **alias != web.search)
            .map(|(alias, web)| (alias.as_str(), web.search.as_str()))
    }

    /// Every command a keyword reaches, the files search Space reaches and every web search,
    /// by title.
    pub fn entries(&self) -> Vec<KeywordEntry> {
        let mut commands: BTreeMap<&CommandId, Vec<String>> = BTreeMap::new();
        for (keyword, command) in &self.keywords {
            commands.entry(command).or_default().push(keyword.clone());
        }
        if let Some(files) = &self.space_on_empty {
            commands.entry(files).or_default();
        }
        let mut entries: Vec<KeywordEntry> = commands
            .into_iter()
            .filter_map(|(id, mut keywords)| {
                let spec = self.commands.get(id)?;
                keywords.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
                Some(KeywordEntry {
                    target: KeywordTarget::Command(id.clone()),
                    title: spec.title().to_owned(),
                    subtitle: spec.presentation.placeholder.clone(),
                    icon: spec.icon.clone(),
                    keywords,
                })
            })
            .collect();
        let mut searches: BTreeMap<&str, KeywordEntry> = BTreeMap::new();
        for (alias, web) in &self.web {
            searches
                .entry(&web.search)
                .or_insert_with(|| KeywordEntry {
                    target: KeywordTarget::Web(web.search.clone()),
                    title: web.title.clone(),
                    subtitle: "Web search".into(),
                    icon: crate::Icon::symbol(WEB_SEARCH_SYMBOL),
                    keywords: Vec::new(),
                })
                .keywords
                .push(alias.clone());
        }
        for mut entry in searches.into_values() {
            let search = match &entry.target {
                KeywordTarget::Web(search) => search.clone(),
                _ => unreachable!("grouped by web search"),
            };
            // The search's own alias first, then its shorthands.
            entry
                .keywords
                .sort_by_key(|alias| (*alias != search, alias.len(), alias.clone()));
            entries.push(entry);
        }
        let mut paths: BTreeMap<&Path, KeywordEntry> = BTreeMap::new();
        for (alias, (path, title)) in &self.paths {
            paths
                .entry(path)
                .or_insert_with(|| KeywordEntry {
                    target: KeywordTarget::Path(path.clone()),
                    title: title.clone(),
                    subtitle: crate::item::tilde(&path.to_string_lossy()),
                    icon: Some(crate::Icon::File(path.clone())),
                    keywords: Vec::new(),
                })
                .keywords
                .push(alias.clone());
        }
        entries.extend(paths.into_values().map(|mut entry| {
            entry
                .keywords
                .sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
            entry
        }));
        entries.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.target.cmp(&b.target)));
        entries
    }

    /// [`Self::entries`] without the commands that are [`CommandSpec::unhinted`].
    pub(crate) fn hinted_entries(&self) -> Vec<KeywordEntry> {
        let mut entries = self.entries();
        entries.retain(|entry| match &entry.target {
            KeywordTarget::Command(id) => self.commands.get(id).is_none_or(|spec| spec.hinted),
            _ => true,
        });
        entries
    }

    pub(crate) fn validate(&self) -> Result<(), CommandError> {
        if !self.commands.contains_key(&self.default) {
            return Err(CommandError::MissingDefault(self.default.clone()));
        }
        if let Some(target) = &self.space_on_empty
            && !self.commands.contains_key(target)
        {
            return Err(CommandError::MissingSpaceTarget(target.clone()));
        }
        Ok(())
    }

    pub(crate) fn default(&self) -> &CommandId {
        &self.default
    }

    pub(crate) fn space_on_empty(&self, current: &CommandId) -> Option<&CommandId> {
        if current == &self.default {
            self.space_on_empty.as_ref()
        } else {
            None
        }
    }

    pub(crate) fn command(&self, id: &CommandId) -> Option<&CommandSpec> {
        self.commands.get(id)
    }

    pub fn ids(&self) -> Vec<String> {
        self.commands.keys().map(|id| id.as_str().to_owned()).collect()
    }

    pub fn keyword(&self, keyword: &str) -> Option<&CommandId> {
        self.keywords.get(keyword)
    }

    /// Every keyword some command answers, web search aliases aside.
    pub fn keywords(&self) -> impl Iterator<Item = &str> {
        self.keywords.keys().map(String::as_str)
    }

    pub(crate) fn contains(&self, id: &CommandId) -> bool {
        self.commands.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> CommandId {
        CommandId::new(value).unwrap()
    }

    #[test]
    fn rejects_duplicate_ids_and_keyword_clashes() {
        let primary = id("sample.search");
        let alternate = id("sample.lookup");
        let mut registry = CommandRegistry::new(primary.clone(), None);
        let spec = || CommandSpec::new(primary.clone(), 10, QueryPresentation::new("Search"));
        registry.register(spec()).unwrap();
        assert_eq!(
            registry.register(spec()),
            Err(CommandError::DuplicateId(primary.clone()))
        );
        registry
            .register(CommandSpec::new(
                alternate.clone(),
                10,
                QueryPresentation::new("Lookup"),
            ))
            .unwrap();
        registry.add_keyword(&primary, "find").unwrap();
        assert_eq!(
            registry.add_keyword(&alternate, "find"),
            Err(CommandError::KeywordConflict {
                keyword: "find".into(),
                command: alternate.clone(),
            })
        );
        assert_eq!(
            registry.add_keyword(&alternate, " "),
            Err(CommandError::InvalidKeyword(" ".into()))
        );
    }

    #[test]
    fn requires_registered_default_and_space_target() {
        let default = id("sample.search");
        let target = id("sample.files");
        let mut registry = CommandRegistry::new(default.clone(), Some(target.clone()));
        assert_eq!(
            registry.validate(),
            Err(CommandError::MissingDefault(default.clone()))
        );
        registry
            .register(CommandSpec::new(default, 10, QueryPresentation::new("Search")))
            .unwrap();
        assert_eq!(registry.validate(), Err(CommandError::MissingSpaceTarget(target)));
    }

    #[test]
    fn session_returns_to_the_default_when_its_active_command_is_removed() {
        let active = id("sample.lookup");
        let mut initial = CommandRegistry::new(id("sample.search"), None);
        for command in [id("sample.search"), active.clone()] {
            initial
                .register(CommandSpec::new(command, 10, QueryPresentation::new("Search")))
                .unwrap();
        }
        initial.add_keyword(&active, "lookup").unwrap();
        let mut session = crate::Session::new(crate::History::default(), initial).unwrap();
        session.set_query("lookup");
        session.intent(crate::Intent::Space);
        let effects = session.set_query("term");
        let generation = match effects.as_slice() {
            [crate::Effect::Search { generation, .. }] => *generation,
            effects => panic!("expected a search effect, got {effects:?}"),
        };

        let default = id("sample.search");
        let recent = id("sample.recent");
        let mut replacement = CommandRegistry::new(default.clone(), None);
        replacement
            .register(CommandSpec::new(
                default.clone(),
                10,
                QueryPresentation::new("Search"),
            ))
            .unwrap();
        replacement
            .register(
                CommandSpec::new(recent.clone(), 10, QueryPresentation::new("Recent"))
                    .with_query_eligibility(QueryEligibility::EmptyOnly),
            )
            .unwrap();
        replacement
            .set_default_providers(vec![default.clone(), recent.clone()], None)
            .unwrap();
        let effects = session.set_commands(replacement).unwrap();
        assert!(matches!(
            effects.as_slice(),
            [crate::Effect::ClearQuery, crate::Effect::Search { command, query, .. }]
                if *command == recent && query.is_empty()
        ));
        assert_eq!(session.command_id(), &default);
        assert!(session.query().is_empty());
        assert!(!session.deliver_from(active, crate::searcher::Delivery::Done { generation }));
    }

    #[test]
    fn command_ids_are_qualified_and_validated() {
        assert_eq!(id("test.search").as_str(), "test.search");
        for invalid in ["search", "Test.search", "test.search.extra", "test."] {
            assert!(CommandId::new(invalid).is_err());
        }
    }

    #[test]
    fn candidate_budgets_default_to_and_never_undercut_display_limits() {
        let mut command = CommandSpec::new(id("rank.search"), 3, QueryPresentation::new("Search"));
        assert_eq!(command.candidate_limit(), 3);
        command = command.with_candidate_limit(8);
        assert_eq!(command.candidate_limit(), 8);
        command.result_limit = 9;
        assert_eq!(command.candidate_limit(), 9);
        let empty = CommandSpec::new(id("empty.search"), 0, QueryPresentation::new("Search"));
        assert_eq!(empty.candidate_limit(), 0);
    }
}
