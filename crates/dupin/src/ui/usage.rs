use crate::sync::UsageUpdate;

use super::Ui;

impl Ui {
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
