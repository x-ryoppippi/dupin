use std::path::{Path, PathBuf};

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
    /// Open the Snapshots window, showing the saved snapshot with this ID if there is one.
    OpenSnapshots(Option<String>),
    /// Run the search the saved snapshot with this ID was taken of again.
    ResumeSnapshot(String),
    /// Show another view of the same command inside the panel.
    Push(Page),
    /// Type the keyword and Space in the main search, as a row the keyword reaches offers;
    /// an empty keyword is Space on an empty query.
    Keyword(String),
    ScriptArguments(PathBuf),
    RunScript(crate::script::ScriptRun),
    /// Run the script config.toml names, by its file name in the scripts folder, with the
    /// row's value as its one argument. Dupin finds the script only when the action runs.
    ConfiguredScript {
        script: String,
        argument: String,
    },
    /// Something only the extension `extension` (its package ID) offered on its own row.
    Extension {
        extension: String,
        request: ExtensionRequest,
    },
}

/// What a chosen extension row asks the host to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtensionRequest {
    /// Run the guest's `actions[action]` with `data`.
    Run {
        action: String,
        data: Option<serde_json::Value>,
    },
    /// Paste the text into the frontmost app while the extension holds `clipboard:paste`, or
    /// copy it. With `template`, its placeholders are filled in first, as a snippet's are.
    Paste { text: String, template: bool },
    /// Grant the scope named `scope` as `revision` declares it, which the row showed.
    Allow { scope: String, revision: String },
    /// Turn the scope off, so the extension stops asking for it.
    Deny { scope: String },
    /// Ask macOS for the access a granted scope still needs.
    RequestAccess { scope: String },
    /// Turn on the extension's capabilities, which it asked for while they were off.
    TurnOn,
    /// Show the extension in Settings, where the user picks what a scope may use.
    Review,
    /// Run the package's kind action `action` on a row Dupin's own sources found, which stands
    /// for `subject`. Dupin hands the package only the field its manifest names for the action,
    /// and only once the user allows it to see that field.
    Kind {
        action: String,
        subject: crate::kind::Subject,
    },
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

/// One line a row shows at its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Accessory {
    Text(String),
    /// Drawn as a label.
    Tag(String),
}

/// What a row's icon shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// The Finder icon of a file, folder or app.
    File(PathBuf),
    /// An image file drawn as it is, such as an extension's or a script's own icon.
    Image(PathBuf),
    /// An image an extension keeps, drawn scaled down.
    Host(HostImage),
    /// An SF Symbol, by a name [`Icon::symbol`] accepted, drawn in the colour of the text beside
    /// it.
    Symbol(String),
}

pub use dupin_plugin_protocol::symbol::{MAX_SYMBOL_NAME, SYMBOL_PREFIX, is_symbol_name};

/// An image an extension's row names by a handle only Dupin resolves, within that package's
/// own storage, so its bytes never travel with the row.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HostImage {
    /// The extension whose storage the handle belongs to, as [`Action::Extension`] names it.
    pub extension: String,
    pub source: HostImageSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HostImageSource {
    /// A blob stored with one of the package's records, by its handle.
    Blob(String),
    /// A file a clipboard watch delivered to the package, by its path and the handle it came
    /// with.
    File { path: PathBuf, handle: String },
}

impl HostImage {
    /// The bytes of text it carries, for budgets on what is kept in memory.
    pub(crate) fn text_bytes(&self) -> usize {
        self.extension.len()
            + match &self.source {
                HostImageSource::Blob(handle) => handle.len(),
                HostImageSource::File { path, handle } => path.as_os_str().len() + handle.len(),
            }
    }
}

impl Icon {
    /// An icon a script or extension named: the image itself when the path names an image
    /// file, otherwise that file's Finder icon.
    pub fn declared(path: PathBuf) -> Self {
        const IMAGES: [&str; 8] = ["png", "jpg", "jpeg", "gif", "svg", "pdf", "icns", "tiff"];
        let image = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| IMAGES.contains(&extension.to_ascii_lowercase().as_str()));
        if image {
            Self::Image(path)
        } else {
            Self::File(path)
        }
    }

    /// The SF Symbol `name`, when [`is_symbol_name`] accepts it. A name the system has no symbol
    /// for draws nothing.
    pub fn symbol(name: &str) -> Option<Self> {
        is_symbol_name(name).then(|| Self::Symbol(name.to_owned()))
    }

    /// The file the icon is drawn from, unless it is an extension's stored image or a symbol.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::File(path) | Self::Image(path) => Some(path),
            Self::Host(_) | Self::Symbol(_) => None,
        }
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
    pub icon: Option<Icon>,
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
    /// A file system entry: Enter opens it, ⌘Enter reveals it. A source gives it the rest of
    /// its kind's actions with [`Kinds`](crate::kind::Kinds).
    pub fn path(path: PathBuf, title: String, matched: Vec<usize>) -> Self {
        let opening = crate::kind::opening(&path);
        Self::path_with(path, title, matched, opening)
    }

    /// A file system entry with the actions its kind resolved.
    pub(crate) fn path_with(
        path: PathBuf,
        title: String,
        matched: Vec<usize>,
        resolved: crate::kind::Resolved,
    ) -> Self {
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
            icon: Some(Icon::File(path.clone())),
            matched,
            completion: None,
            score: None,
            primary: resolved.primary,
            alternate: resolved.alternate,
            actions: resolved.actions,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_names_are_ascii_words_joined_by_dots() {
        for name in ["moon.fill", "lock", "circle.lefthalf.filled", "1.circle", "a"] {
            assert_eq!(Icon::symbol(name), Some(Icon::Symbol(name.into())), "{name}");
        }
        let long = "a".repeat(MAX_SYMBOL_NAME + 1);
        for name in [
            "",
            ".moon",
            "moon.",
            "moon..fill",
            "moon fill",
            "moon/fill",
            "../moon",
            "moon-fill",
            "moon_fill",
            "\u{ff4d}oon",
            long.as_str(),
        ] {
            assert_eq!(Icon::symbol(name), None, "{name:?}");
        }
        assert!(Icon::symbol(&"a".repeat(MAX_SYMBOL_NAME)).is_some());
        assert_eq!(Icon::Symbol("moon".into()).path(), None);
    }
}
