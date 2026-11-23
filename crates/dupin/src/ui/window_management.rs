use dupin_core::window_management::Layout as WindowLayout;
use dupin_mac::{
    accessibility,
    window_management::{self, Error as WindowError},
};

use super::{Busy, Ui};

impl Ui {
    pub(super) fn schedule_window_action(&mut self, layout: WindowLayout) {
        if self.window_management_enabled && !rune::tooling::is_headless() {
            self.window_action_after_close =
                Some((layout, self.frontmost_pid, Busy::new(self.running.clone())));
        }
    }

    pub(super) fn arrange_window_after_close(&mut self) {
        if let Some((layout, pid, busy)) = self.window_action_after_close.take()
            && self.window_management_enabled
        {
            self.arrange_window(layout, pid, busy);
        }
    }

    fn arrange_window(&mut self, layout: WindowLayout, pid: Option<i32>, busy: Busy) {
        let me = self.me.clone();
        if self.window_action_running {
            me.update(|ui, tx| ui.show_notice("A window command is already running.".into(), tx));
            return;
        }
        self.window_action_running = true;
        let displays = window_management::displays();
        std::thread::spawn(move || {
            let result = pid
                .ok_or(WindowError::NoWindow)
                .and_then(|pid| window_management::arrange(pid, layout, &displays));
            me.update(move |ui, tx| {
                ui.window_action_running = false;
                ui.window_permission_required = ui.window_management_enabled
                    && matches!(
                        result,
                        Err(WindowError::PermissionRequested | WindowError::PermissionRequired)
                    )
                    && !accessibility::is_trusted();
                if !ui.window_management_enabled || ui.picker.is_some() {
                    return;
                }
                match result {
                    Err(WindowError::PermissionRequired) if ui.window_permission_required => ui.show(tx),
                    Err(WindowError::PermissionRequested | WindowError::PermissionRequired) => {}
                    Err(error) => ui.show_notice(error.message().into(), tx),
                    Ok(()) => {}
                }
            });
            drop(busy);
        });
    }
}
