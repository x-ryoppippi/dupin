use rune::Tx;

use super::Ui;

impl Ui {
    pub(crate) fn calendar_access_changed(&mut self, tx: &mut Tx) {
        // The bundled extension reads calendars through the broker, which must know first.
        crate::extensions::access::refresh_os_access(&self.extensions.bridge, &self.calendar_access);
        if self.session.command_id().as_str() == crate::features::calendar::CALENDAR {
            let effects = self.session.refresh();
            self.apply(effects, tx);
        }
    }
}
