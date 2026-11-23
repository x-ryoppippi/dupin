use std::path::PathBuf;

use dupin_core::item::Transfer;

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
