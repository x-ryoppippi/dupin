use std::collections::HashMap;
use std::fmt;

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
    pub result_limit: usize,
    candidate_limit: Option<usize>,
    fallback: bool,
    pub private_results: bool,
    pub presentation: QueryPresentation,
    pub deadline: std::time::Duration,
    pub(crate) query_eligibility: QueryEligibility,
}

impl CommandSpec {
    pub fn new(id: CommandId, result_limit: usize, presentation: QueryPresentation) -> Self {
        Self {
            id,
            result_limit,
            candidate_limit: None,
            fallback: false,
            private_results: false,
            presentation,
            deadline: std::time::Duration::from_secs(30),
            query_eligibility: QueryEligibility::NonEmpty,
        }
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
            Self::MissingDefault(id) => write!(f, "default command `{id}` is not registered"),
            Self::MissingSpaceTarget(id) => write!(f, "Space target `{id}` is not registered"),
        }
    }
}

impl std::error::Error for CommandError {}

#[derive(Clone, Debug)]
pub struct CommandRegistry {
    default: CommandId,
    space_on_empty: Option<CommandId>,
    destination_search: Option<CommandId>,
    commands: HashMap<CommandId, CommandSpec>,
    keywords: HashMap<String, CommandId>,
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

    pub fn add_keyword(
        &mut self,
        command: &CommandId,
        keyword: impl Into<String>,
    ) -> Result<(), CommandError> {
        if !self.commands.contains_key(command) {
            return Err(CommandError::UnknownCommand(command.clone()));
        }
        let keyword = keyword.into();
        if keyword.trim().is_empty() || keyword.trim() != keyword {
            return Err(CommandError::InvalidKeyword(keyword));
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
