use std::path::PathBuf;

pub use dupin_plugin_protocol::View;

/// What running a result does. Providers decide; the platform layer carries it out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open with the default handler: launch an app, open a document or folder.
    Open(PathBuf),
    ChooseApplication(PathBuf),
    OpenWith {
        path: PathBuf,
        application: PathBuf,
    },
    Trash(PathBuf),
    /// Show in the file manager, selected.
    Reveal(PathBuf),
    /// Open a URL with its registered handler.
    OpenUrl(String),
    Copy(String),
    /// Show the path in the platform's native preview UI.
    Preview(PathBuf),
    /// Open the host application's preferences.
    OpenPreferences,
    ScriptArguments(PathBuf),
    RunScript(crate::script::ScriptRun),
}

/// Whether an item action completes the pick or leaves the current search available.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ActionDisposition {
    /// Run the action, record usage and dismiss the launcher.
    #[default]
    Dismiss,
    /// Run the action without recording a pick or dismissing the launcher.
    KeepOpen,
    Consume,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirmation {
    pub prompt: String,
    pub button: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemAction {
    pub id: String,
    pub title: String,
    pub(crate) action: Action,
    pub(crate) disposition: ActionDisposition,
    pub(crate) confirmation: Option<Confirmation>,
}

impl ItemAction {
    pub fn new(id: &str, title: &str, action: Action) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            action,
            disposition: ActionDisposition::Dismiss,
            confirmation: None,
        }
    }

    /// Leave the current search open after this action runs.
    pub fn keep_open(mut self) -> Self {
        self.disposition = ActionDisposition::KeepOpen;
        self
    }

    pub fn confirm(mut self, prompt: &str, button: &str) -> Self {
        self.confirmation = Some(Confirmation {
            prompt: prompt.into(),
            button: button.into(),
        });
        self
    }

    pub fn confirmation(&self) -> Option<&Confirmation> {
        self.confirmation.as_ref()
    }

    pub fn consumes(mut self) -> Self {
        self.disposition = ActionDisposition::Consume;
        self
    }
}

/// One candidate row, as every provider returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Unique among the results of one search; the UI keys rows by it.
    pub id: String,
    pub view: View,
    /// Participate in usage recording and history ranking.
    pub record_usage: bool,
    /// The file whose icon represents the row, if any.
    pub icon: Option<PathBuf>,
    /// Character indices in `display_title()` that matched the query, for highlighting.
    pub matched: Vec<usize>,
    /// Text to put in the query field when the user completes this result.
    pub completion: Option<String>,
    /// How well the item matched, for providers that rank their results; the history adds
    /// to it. None for results that keep a provider's own order.
    pub score: Option<i32>,
    /// The action ID for Enter; without one, Enter offers the action menu.
    pub primary: Option<String>,
    /// The action ID for ⌘Enter.
    pub alternate: Option<String>,
    pub actions: Vec<ItemAction>,
}

impl Item {
    /// A file system entry: Enter opens it, ⌘Enter reveals it.
    pub fn path(path: PathBuf, title: String, matched: Vec<usize>) -> Self {
        let subtitle = path
            .parent()
            .map(|p| tilde(&p.to_string_lossy()))
            .unwrap_or_default();
        Item {
            id: path.to_string_lossy().into_owned(),
            view: View::Text { title, subtitle },
            record_usage: true,
            icon: Some(path.clone()),
            matched,
            completion: None,
            score: None,
            primary: Some("open".into()),
            alternate: Some("reveal".into()),
            actions: vec![
                ItemAction::new("open", "Open", Action::Open(path.clone())),
                ItemAction::new("reveal", "Reveal in Finder", Action::Reveal(path.clone())),
                ItemAction::new(
                    "copy-path",
                    "Copy path",
                    Action::Copy(path.to_string_lossy().into_owned()),
                ),
            ],
        }
    }

    pub fn display_title(&self) -> &str {
        match &self.view {
            View::Text { title, .. } | View::Detail { title, .. } => title,
            View::Pair { source, .. } => source,
        }
    }

    pub fn action(&self, id: &str) -> Option<&Action> {
        self.actions
            .iter()
            .find(|action| action.id == id)
            .map(|action| &action.action)
    }

    pub fn item_action(&self, id: &str) -> Option<&ItemAction> {
        self.actions.iter().find(|action| action.id == id)
    }
}

/// `$HOME/x` as `~/x`, as Finder and shells show it.
pub fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            let rest = &path[home.len()..];
            if rest.is_empty() || rest.starts_with('/') {
                format!("~{rest}")
            } else {
                path.to_owned()
            }
        }
        _ => path.to_owned(),
    }
}
