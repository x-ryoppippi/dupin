use std::path::PathBuf;

/// What running a result does. Providers decide; the platform layer carries it out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open with the default handler: launch an app, open a document or folder.
    Open(PathBuf),
    /// Show in the file manager, selected.
    Reveal(PathBuf),
    /// Open a URL with its registered handler.
    OpenUrl(String),
    Copy(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemAction {
    pub id: String,
    pub title: String,
    pub action: Action,
}

impl ItemAction {
    pub fn new(id: &str, title: &str, action: Action) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            action,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Application,
    File,
    Dictionary,
}

/// One candidate row, as every provider returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Unique among the results of one search; the UI keys rows by it.
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub kind: ItemKind,
    /// The file whose icon represents the row, if any.
    pub icon: Option<PathBuf>,
    /// Character indices in `title` that matched the query, for highlighting.
    pub matched: Vec<usize>,
    /// How well the item matched, for providers that rank their results; the history adds
    /// to it. None for results that arrive in a provider's own order (Spotlight's).
    pub score: Option<i32>,
    /// The action ID for Enter; without one, Enter offers the action menu.
    pub primary: Option<String>,
    /// The action ID for ⌘Enter.
    pub alternate: Option<String>,
    pub actions: Vec<ItemAction>,
}

impl Item {
    /// A file system entry: Enter opens it, ⌘Enter reveals it.
    pub fn path(kind: ItemKind, path: PathBuf, title: String, matched: Vec<usize>) -> Self {
        let subtitle = path
            .parent()
            .map(|p| tilde(&p.to_string_lossy()))
            .unwrap_or_default();
        Item {
            id: path.to_string_lossy().into_owned(),
            title,
            subtitle,
            kind,
            icon: Some(path.clone()),
            matched,
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

    pub fn action(&self, id: &str) -> Option<&Action> {
        self.actions
            .iter()
            .find(|action| action.id == id)
            .map(|action| &action.action)
    }
}

/// `$HOME/x` as `~/x`, as Finder and shells show it.
fn tilde(path: &str) -> String {
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
