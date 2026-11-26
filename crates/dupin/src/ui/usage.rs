use crate::control::SyncAction;
use crate::sync::UsageUpdate;

use super::Ui;

impl Ui {
    /// A Settings → Sync action, answered back to Settings.
    pub(crate) fn sync_request(&self, action: SyncAction) {
        let prefs = self.prefs.clone();
        self.sync.request(action, move |result| {
            prefs.update(move |s, _| s.sync_answered(result))
        });
    }

    /// Usage history from the sync worker: this Mac's saved log, the other Macs', or a removed
    /// Mac's to fold into this one's. Ranking takes it from the next search on.
    pub(super) fn usage_changed(&mut self, update: UsageUpdate) {
        match update {
            UsageUpdate::Loaded { own, others } => {
                self.session.set_others(others);
                let picked_before_loading = !self.session.own_history().is_empty();
                self.session.load_own(&own);
                if picked_before_loading {
                    self.sync.save_usage(self.session.own_history());
                }
            }
            UsageUpdate::Restarted { others } => {
                self.session.restart_own();
                self.session.set_others(others);
            }
            UsageUpdate::Others(others) => self.session.set_others(others),
            UsageUpdate::Absorb(other) => {
                self.session.absorb(&other);
                self.sync.save_usage(self.session.own_history());
            }
        }
    }
}
