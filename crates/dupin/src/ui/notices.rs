use std::path::PathBuf;

use dupin_core::item::Transfer;
use rune::Tx;

use super::Ui;

#[derive(PartialEq, Eq)]
pub(super) enum FileAction {
    Rename,
    Transfer(Transfer),
    Duplicate,
    Compress,
    Trash,
}

#[derive(PartialEq, Eq)]
pub(super) struct FileOperation {
    pub(super) action: FileAction,
    pub(super) path: PathBuf,
}

enum Lifetime {
    Persistent(Option<FileOperation>),
    Timed(f64),
}

struct Notice {
    message: String,
    lifetime: Lifetime,
}

#[derive(Default)]
pub(super) struct Notices(Option<Notice>);

impl Notices {
    pub(super) fn message(&self) -> Option<&str> {
        self.0.as_ref().map(|notice| notice.message.as_str())
    }

    pub(super) fn clear(&mut self) {
        self.0 = None;
    }

    pub(super) fn persistent(&mut self, message: String, owner: Option<FileOperation>) {
        self.0 = Some(Notice {
            message,
            lifetime: Lifetime::Persistent(owner),
        });
    }

    pub(super) fn timed(&mut self, message: String, owner: Option<&FileOperation>, until: f64) -> bool {
        if let Some(Notice {
            lifetime: Lifetime::Persistent(current),
            ..
        }) = &self.0
            && (owner.is_none() || owner != current.as_ref())
        {
            return false;
        }
        self.0 = Some(Notice {
            message,
            lifetime: Lifetime::Timed(until),
        });
        true
    }

    pub(super) fn expire(&mut self, clock: f64) -> bool {
        if matches!(&self.0, Some(Notice { lifetime: Lifetime::Timed(until), .. }) if clock >= *until) {
            self.clear();
            return true;
        }
        false
    }
}

impl Ui {
    pub fn show_notice(&mut self, message: String, tx: &mut Tx) {
        self.show_persistent_notice(message, None, tx);
    }

    pub(super) fn show_persistent_notice(
        &mut self,
        message: String,
        owner: Option<FileOperation>,
        tx: &mut Tx,
    ) {
        self.notices.persistent(message, owner);
        tx.instant();
        self.show(tx);
    }

    pub fn notice(&self) -> Option<&str> {
        self.notices.message()
    }

    pub(super) fn timed_notice(&mut self, notice: String, tx: &mut Tx) {
        self.show_timed_notice(notice, None, tx);
    }

    pub(super) fn show_timed_notice(&mut self, message: String, owner: Option<&FileOperation>, tx: &mut Tx) {
        let until = tx.clock() + 5.0;
        if self.notices.timed(message, owner, until) {
            tx.wake_at(until);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_successful_retry_replaces_its_failure_with_an_expiring_notice() {
        let mut notices = Notices::default();
        let operation = || FileOperation {
            action: FileAction::Duplicate,
            path: "report.txt".into(),
        };
        notices.persistent("Couldn't duplicate".into(), Some(operation()));

        assert!(notices.timed("Duplicated".into(), Some(&operation()), 5.0));
        assert_eq!(notices.message(), Some("Duplicated"));
        assert!(notices.expire(5.0));
        assert_eq!(notices.message(), None);
    }

    #[test]
    fn successes_leave_unrelated_persistent_notices_visible() {
        let mut notices = Notices::default();
        let operation = FileOperation {
            action: FileAction::Duplicate,
            path: "report.txt".into(),
        };
        notices.persistent("Couldn't register hotkey".into(), None);
        assert!(!notices.timed("Duplicated".into(), Some(&operation), 5.0));
        assert_eq!(notices.message(), Some("Couldn't register hotkey"));

        notices.persistent(
            "Couldn't duplicate another file".into(),
            Some(FileOperation {
                action: FileAction::Duplicate,
                path: "other.txt".into(),
            }),
        );
        assert!(!notices.timed("Duplicated".into(), Some(&operation), 5.0));
        assert_eq!(notices.message(), Some("Couldn't duplicate another file"));

        notices.persistent(
            "Couldn't compress".into(),
            Some(FileOperation {
                action: FileAction::Compress,
                path: "report.txt".into(),
            }),
        );
        assert!(!notices.timed("Duplicated".into(), Some(&operation), 5.0));
        assert!(!notices.timed("Snapshot saved".into(), None, 5.0));
        assert_eq!(notices.message(), Some("Couldn't compress"));
    }
}
