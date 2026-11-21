use std::path::PathBuf;

/// What running a result does. Providers decide; the platform layer carries it out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open with the default handler: launch an app, open a document or folder.
    Open(PathBuf),
    /// Show in the file manager, selected.
    Reveal(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Application,
    File,
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
    /// Enter.
    pub primary: Action,
    /// ⌘Enter.
    pub alternate: Option<Action>,
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
            primary: Action::Open(path.clone()),
            alternate: Some(Action::Reveal(path)),
        }
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
