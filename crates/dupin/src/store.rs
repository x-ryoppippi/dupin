//! The history file: `~/Library/Application Support/Dupin/history.json` on macOS,
//! `$XDG_STATE_HOME/dupin/history.json` on Linux.

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};

use dupin_core::History;

/// `~/Library/Application Support/Dupin`, where Dupin keeps what it learns about its user.
#[cfg(target_os = "macos")]
pub fn support_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support/Dupin"))
}

/// `$XDG_STATE_HOME/dupin`, where Dupin keeps what it learns about its user.
#[cfg(target_os = "linux")]
pub fn support_dir() -> Option<PathBuf> {
    crate::platform::dirs::state()
}

#[cfg(target_os = "macos")]
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(|base| PathBuf::from(base).join("dupin"))
        .or_else(support_dir)
}

/// `$XDG_DATA_HOME/dupin`, or `~/.local/share/dupin`.
#[cfg(target_os = "linux")]
pub fn data_dir() -> Option<PathBuf> {
    crate::platform::dirs::data()
}

pub const FILE: &str = "history.json";

fn path() -> Option<PathBuf> {
    support_dir().map(|dir| dir.join(FILE))
}

/// The saved history, or an empty one (saying why, when the file exists but can't be used).
pub fn load() -> History {
    let Some(path) = path() else {
        return History::default();
    };
    let src = match std::fs::read_to_string(&path) {
        Ok(src) => src,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return History::default(),
        Err(e) => {
            crate::log::warn(&format!(
                "couldn't read {}: {e}; starting with no history",
                path.display()
            ));
            return History::default();
        }
    };
    History::from_json(&src).unwrap_or_else(|e| {
        crate::log::warn(&format!(
            "{} is invalid ({e}); starting with no history",
            path.display()
        ));
        History::default()
    })
}

/// Writes history snapshots on one thread of its own, so a save never touches the disk on the
/// UI thread, and snapshots that pile up while one is written collapse to the newest.
pub struct Saver {
    snapshots: Sender<String>,
}

impl Saver {
    pub fn spawn() -> Saver {
        let (snapshots, rx) = mpsc::channel::<String>();
        std::thread::Builder::new()
            .name("dupin-history".into())
            .spawn(move || {
                while let Ok(mut json) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        json = newer;
                    }
                    if let Err(e) = write(&json) {
                        crate::log::warn(&format!("couldn't save the history: {e}"));
                    }
                }
            })
            .expect("spawn history writer");
        Saver { snapshots }
    }

    pub fn save(&self, history: &History) {
        let _ = self.snapshots.send(history.to_json());
    }
}

/// Write beside the file and rename over it, so a crash mid-write leaves the old file whole.
fn write(json: &str) -> std::io::Result<()> {
    use std::io::Write;

    let path = path().ok_or_else(|| std::io::Error::other("HOME is unset"))?;
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent directory"))?;
    #[cfg(target_os = "macos")]
    std::fs::create_dir_all(dir)?;
    #[cfg(target_os = "linux")]
    crate::platform::dirs::create_private(dir)?;
    let mut staged = tempfile::Builder::new()
        .prefix(".history.json.")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    staged.write_all(json.as_bytes())?;
    staged.persist(path).map(drop).map_err(|error| error.error)
}
