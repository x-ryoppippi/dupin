use dupin_mac::CalendarProvider;
use rune::AppHandle;

use super::Ui;

pub(super) fn provider(me: &AppHandle<Ui>) -> CalendarProvider {
    if rune::tooling::is_headless() {
        dupin_mac::CalendarProvider::fixtures()
    } else {
        let panel = me.clone();
        dupin_mac::CalendarProvider::new(move || {
            panel.update(|ui, tx| {
                if ui.session.command_id().as_str() == crate::commands::CALENDAR {
                    let effects = ui.session.refresh();
                    ui.apply(effects, tx);
                }
            });
        })
    }
}
