use std::path::PathBuf;

pub use dupin_plugin_protocol::View;

/// A view a provider opens over its own results. The session searches the same command for
/// it, and the provider tells the views apart by `view`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub view: String,
    /// Shown beside the command's chip while the view is open.
    pub title: String,
    /// Opaque to the session: what the provider asked to be given back with each search.
    pub context: Option<serde_json::Value>,
}

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
    /// List the folder's entries in the launcher.
    Browse(PathBuf),
    /// Ask for a new name in the query field; the session then renames the entry.
    Rename(PathBuf),
    /// Search for a destination folder before transferring the entry.
    ChooseDestination {
        path: PathBuf,
        transfer: Transfer,
    },
    /// Copy beside the original under a name that is not taken.
    Duplicate(PathBuf),
    /// Zip beside the original under a name that is not taken.
    Compress(PathBuf),
    System(SystemCommand),
    /// Show in the file manager, selected.
    Reveal(PathBuf),
    /// Open a URL with its registered handler.
    OpenUrl(String),
    Copy(String),
    Share {
        target: ShareTarget,
        paths: Vec<PathBuf>,
    },
    CopyFiles(Vec<PathBuf>),
    /// Put a clipboard history entry back on the clipboard as it was copied, then, with
    /// `paste`, paste it into the frontmost app.
    RestoreClipboardEntry {
        entry: u64,
        paste: bool,
    },
    /// Remove one entry from the clipboard history; the system clipboard keeps its contents.
    DeleteClipboardEntry(u64),
    ClearClipboardHistory,
    /// Read the clipboard once so macOS asks whether Dupin may; the contents are discarded.
    RequestClipboardAccess,
    /// Copy a snippet's text with its placeholders filled in as it runs, then, with `paste`,
    /// paste it into the frontmost app.
    Snippet {
        text: String,
        paste: bool,
    },
    /// Show the path in the platform's native preview UI.
    Preview(PathBuf),
    /// Open the host application's preferences.
    OpenPreferences,
    /// Show another view of the same command inside the panel.
    Push(Page),
    ScriptArguments(PathBuf),
    RunScript(crate::script::ScriptRun),
    ArrangeWindow(crate::window_management::Layout),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    Move,
    Copy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemCommand {
    Sleep,
    LockScreen,
    SleepDisplays,
    Restart,
    ShutDown,
    LogOut,
    EmptyTrash,
    ToggleDarkMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareTarget {
    Picker,
    Service { name: String, label: String },
}

/// The primary values a result can pass to other actions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Payload {
    pub text: Option<String>,
    pub paths: Vec<PathBuf>,
}

impl Payload {
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            text: Some(value.into()),
            paths: Vec::new(),
        }
    }

    pub fn nonblank_text(&self) -> Option<&str> {
        self.text.as_deref().filter(|text| !text.trim().is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.nonblank_text().is_none()
    }
}

/// Whether an item action completes the pick or leaves the current search available.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ActionDisposition {
    /// Run the action, record usage and dismiss the launcher.
    #[default]
    Dismiss,
    /// Run the action without dismissing the launcher.
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
    pub payload: Payload,
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
            payload: Payload {
                text: None,
                paths: vec![path.clone()],
            },
            record_usage: true,
            icon: Some(path.clone()),
            matched,
            completion: None,
            score: None,
            primary: Some("open".into()),
            alternate: Some("reveal".into()),
            actions: vec![
                ItemAction::new("open", "Open", Action::Open(path.clone())),
                ItemAction::new("reveal", "Reveal in Finder", Action::Reveal(path)),
            ],
        }
    }

    pub fn has_actions(&self) -> bool {
        !self.actions.is_empty() || !self.payload.is_empty()
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
