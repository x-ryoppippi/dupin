use std::sync::atomic::Ordering;

use rune::Tx;

use super::{Busy, Ui};

impl Ui {
    pub fn snapshot_notice(&mut self, message: String, tx: &mut Tx) {
        self.timed_notice(message, tx);
    }

    pub fn show_snapshots(&mut self, tx: &mut Tx) {
        if self.me.is_visible() {
            self.snapshots_after_close = true;
            self.dismiss(tx);
        } else {
            self.snapshots.update(|s, _| s.show());
        }
    }

    pub fn control_snapshots_open(&mut self, call: crate::control::Call, tx: &mut Tx) {
        if call.cancelled.load(Ordering::Relaxed) {
            return;
        }
        if self.picker.is_some() {
            call.finish(Err("Busy: picker active".into()));
        } else {
            self.snapshots.update(move |s, _| s.accept_open(call));
            self.show_snapshots(tx);
        }
    }

    pub fn resume_snapshot(&mut self, saved: dupin_core::snapshots::Snapshot, tx: &mut Tx) {
        match self.session.restore(&saved) {
            Ok(effects) => {
                self.action_screen = None;
                self.apply(effects, tx);
                self.show(tx);
            }
            Err(problem) => self.snapshots.update(move |s, _| s.report(problem)),
        }
    }

    pub(super) fn capture_snapshot(&mut self, tx: &mut Tx) {
        match self.session.capture() {
            Ok(saved) => {
                crate::trace::snapshot("capture accepted");
                let busy = Busy::new(self.running.clone());
                self.snapshots.update(move |s, _| s.save(saved, busy));
                self.snapshot_notice("Saving snapshot…".into(), tx);
            }
            Err(problem) => {
                crate::trace::snapshot("capture refused");
                self.snapshot_notice(problem, tx);
            }
        }
    }
}
