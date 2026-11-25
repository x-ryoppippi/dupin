//! Which of Dupin's own encrypted stores couldn't be opened, and why, for Settings to show on
//! every page. Such a store keeps what it holds untouched and saves nothing until it opens:
//! nothing falls back to a file in the clear.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock, PoisonError};

type Listener = Box<dyn Fn() + Send + Sync>;

static PROBLEMS: Mutex<BTreeMap<&'static str, String>> = Mutex::new(BTreeMap::new());
static LISTENER: OnceLock<Listener> = OnceLock::new();

/// Records whether `store`, named as Settings shows it, opened, and tells the listener when that
/// changed.
pub fn report(store: &'static str, problem: Option<&str>) {
    let changed = {
        let mut problems = PROBLEMS.lock().unwrap_or_else(PoisonError::into_inner);
        match problem {
            Some(problem) => problems.insert(store, problem.to_owned()).as_deref() != Some(problem),
            None => problems.remove(store).is_some(),
        }
    };
    if changed && let Some(listener) = LISTENER.get() {
        listener();
    }
}

/// One sentence per store that couldn't be opened.
pub fn problems() -> Vec<String> {
    PROBLEMS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(store, problem)| {
            format!("{store} couldn't be opened, so nothing is saved there until it can: {problem}.")
        })
        .collect()
}

/// Calls `listener` on the reporting thread whenever the problems change. Only the first
/// listener is kept.
pub fn on_change(listener: impl Fn() + Send + Sync + 'static) {
    let _ = LISTENER.set(Box::new(listener));
}
