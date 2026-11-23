use rune::Tx;

use super::Ui;

impl Ui {
    pub(crate) fn calendar_access_changed(&mut self, tx: &mut Tx) {
        if self.session.command_id().as_str() == crate::commands::CALENDAR {
            let effects = self.session.refresh();
            self.apply(effects, tx);
        }
    }
}
