use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::platform::{accessibility, quick_look::Previewer, window::DisplayNotice};
use dupin_core::{DisplaySelection, InputSource, Position};
#[cfg(target_os = "macos")]
use objc2_foundation::NSRect;
use rune::raw_window_handle::RawWindowHandle;
use rune::{Align, DisplayTarget, OverlayPlacement, ScreenEdge, Tx};

use super::{PendingPaste, Ui, selection::SelectionRequest};

// Match Spotlight's offset for the top position.
const TOP_MARGIN: f32 = 180.0;
/// How long, in seconds, the panel may be without focus before that counts as losing it.
#[cfg(target_os = "linux")]
const FOCUS_SETTLE: f64 = 0.15;

pub struct Busy(Arc<AtomicUsize>);

impl Busy {
    pub fn new(running: Arc<AtomicUsize>) -> Self {
        running.fetch_add(1, Ordering::AcqRel);
        Self(running)
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Restart {
    Update,
    Requested,
}

impl Restart {
    fn waiting_for_work(self, actions: &AtomicUsize, scripts: &AtomicUsize) -> bool {
        actions.load(Ordering::Acquire) > 0 || (self == Self::Update && scripts.load(Ordering::Acquire) > 0)
    }
}

#[test]
fn only_automatic_restart_waits_for_scripts() {
    let actions = AtomicUsize::new(0);
    let scripts = AtomicUsize::new(1);
    assert!(Restart::Update.waiting_for_work(&actions, &scripts));
    assert!(!Restart::Requested.waiting_for_work(&actions, &scripts));

    actions.store(1, Ordering::Release);
    assert!(Restart::Requested.waiting_for_work(&actions, &scripts));
}

impl Ui {
    /// The hotkey: show the panel, or hide it when it already shows.
    pub(super) fn toggle(&mut self, tx: &mut Tx) {
        if self.picker.is_some() {
            self.finish_pick(None, tx);
            return;
        }
        let visible = self.me.is_visible();
        crate::trace::panel("toggle", visible, self.shown);
        if visible {
            self.dismiss(tx);
        } else {
            self.show(tx);
        }
    }

    pub(super) fn show(&mut self, tx: &mut Tx) {
        if self.selection_request == SelectionRequest::Reading {
            self.selection_request = SelectionRequest::Superseded;
        }
        crate::trace::summoned();
        #[cfg(target_os = "linux")]
        {
            self.focus_lost_until = None;
            self.focus_wanted_until = Some(tx.clock() + FOCUS_SETTLE);
            tx.wake_at(tx.clock() + FOCUS_SETTLE);
        }
        if !self.shown && !rune::tooling::is_headless() {
            self.front.capture();
            crate::extensions::access::refresh_os_access(&self.extensions.bridge, &self.calendar_access);
        }
        if let Some(problem) = self.backdrop.refresh() {
            crate::log::warn(problem);
        }
        self.query.focused = true;
        if !self.shown {
            self.place_panel();
        }
        self.shown = true;
        self.shown_at = tx.clock();
        self.force_input_source();
        self.me.show();
        if self.session.query().is_empty() {
            self.refresh_default(tx);
        }
    }

    fn place_panel(&mut self) {
        self.display_notice = None;
        let (display, align, inset, margin) = if !rune::tooling::is_headless()
            && let Some(native) = self.native_placement()
        {
            native
        } else {
            let display = match self.display {
                DisplaySelection::Mouse => DisplayTarget::UnderPointer,
                DisplaySelection::Main | DisplaySelection::Active => DisplayTarget::Primary,
            };
            (display, Align::Center, 0.0, self.fallback_margin())
        };
        self.me.place(
            display,
            OverlayPlacement::Edge {
                edge: ScreenEdge::Top,
                margin,
                align,
                inset,
            },
        );
    }

    pub fn open_accessibility_settings(&mut self, tx: &mut Tx) {
        if rune::tooling::is_headless()
            || (!self.selection_permission_required
                && self.display_notice != Some(DisplayNotice::AccessibilityRequired))
        {
            return;
        }
        self.accessibility_after_close = true;
        self.dismiss(tx);
    }

    /// The panel's place on the screen AppKit reports for the selected display.
    #[cfg(target_os = "macos")]
    fn native_placement(&mut self) -> Option<(DisplayTarget, Align, f32, f32)> {
        let screen = crate::platform::window::display_geometry(self.display)?;
        self.display_notice = screen.notice;
        let (inset, margin) = panel_offsets(
            screen.frame,
            screen.visible_frame,
            self.theme.panel_width,
            self.position,
        );
        Some((
            DisplayTarget::Native(rune::NativeDisplayId::MacOS(screen.native_id)),
            Align::Start,
            inset,
            margin,
        ))
    }

    /// Linux places the panel through Rune's display targets.
    #[cfg(target_os = "linux")]
    fn native_placement(&mut self) -> Option<(DisplayTarget, Align, f32, f32)> {
        None
    }

    #[cfg(target_os = "macos")]
    fn fallback_margin(&self) -> f32 {
        TOP_MARGIN
    }

    /// The top margin for `position`, with the center measured on the display under the
    /// pointer.
    #[cfg(target_os = "linux")]
    fn fallback_margin(&self) -> f32 {
        match self.position {
            Position::Top => TOP_MARGIN,
            Position::Center => crate::platform::window::pointer_display_height().map_or(TOP_MARGIN, |h| {
                ((h as f32 - crate::app::HEADER_HEIGHT) / 2.0).max(0.0)
            }),
        }
    }

    fn force_input_source(&mut self) {
        use crate::platform::input_source::{Target, select};
        if self.restore_input.is_some() {
            return;
        }
        let target = match &self.input_source {
            InputSource::Keep => return,
            InputSource::Ascii => Target::Ascii,
            InputSource::Id(id) => Target::Id(id.clone()),
        };
        #[cfg(target_os = "linux")]
        if matches!(target, Target::Id(_)) {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                crate::log::warn(
                    "input_source applies only on macOS; the panel keeps the current input method",
                )
            });
        }
        self.restore_input = select(&target);
    }

    /// Hide and reset, as Esc and losing focus do.
    pub fn dismiss(&mut self, tx: &mut Tx) {
        if self.picker.is_some() {
            self.finish_pick(None, tx);
            return;
        }
        if !self.shown {
            return;
        }
        let effects = self.session.reset();
        self.hide(tx);
        self.apply(effects, tx);
    }

    pub(super) fn prepare_settings(&mut self) {
        crate::trace::settings("requested");
        self.settings_after_close = true;
    }

    pub fn closed(&mut self, tx: &mut Tx) {
        crate::trace::panel("closed", self.me.is_visible(), self.shown);
        let show_snapshots = std::mem::take(&mut self.snapshots_after_close);
        let show_settings = std::mem::take(&mut self.settings_after_close);
        if std::mem::take(&mut self.accessibility_after_close) {
            let me = self.me.clone();
            std::thread::spawn(move || {
                if !accessibility::is_trusted() && accessibility::request_once() {
                    return;
                }
                let settings =
                    crate::platform::workspace::Action::OpenUrl(accessibility::SETTINGS_URL.into());
                if let Err(error) = crate::platform::workspace::run(&settings) {
                    me.update(move |ui, tx| ui.show_notice(error, tx));
                }
            });
        }
        match self.paste_after_close.take() {
            Some(PendingPaste::Snippet(snippet)) => self.deliver_snippet(snippet, true, tx),

            Some(PendingPaste::Clipboard { permit }) => self.paste_clipboard(&permit),
            Some(PendingPaste::Extension { text, permit }) => {
                let paste = self.extensions.bridge.validate(&permit);
                self.hand_over(text, paste, tx);
            }
            None => {}
        }
        if let Some(notice) = self.notice_after_close.take() {
            self.show_notice(notice, tx);
        }
        if let Some(id) = self.script_output_after_close.take() {
            self.show_script_output(id);
        }
        if show_snapshots {
            self.snapshots.update(|s, _| s.show());
        }
        if show_settings {
            crate::trace::settings("panel closed");
            self.prefs.open();
        }
        if !show_snapshots && !show_settings && self.relaunch.is_some() {
            // Rune clears the panel's visibility after this callback.
            self.me.update(Self::resume_restart);
        }
    }

    pub(super) fn hide(&mut self, tx: &mut Tx) {
        self.script_entry = None;
        self.script_card = None;
        crate::trace::panel("hide", self.me.is_visible(), self.shown);
        self.action_screen = None;
        self.sharing.close();
        if let Some(preview) = self.quick_look.borrow().as_ref() {
            preview.close();
        }
        if let Some(previous) = self.restore_input.take() {
            previous.restore();
        }
        self.query.clear();
        self.shown = false;
        self.notices.clear();
        self.selection_permission_required = false;
        tx.close_window();
        if !self.settings_after_close
            && !self.snapshots_after_close
            && self.script_output_after_close.is_none()
            && let Some(request) = self.relaunch.take()
        {
            self.relaunch_when_idle(request);
        }
    }

    /// An update is staged: restart into it now, or once neither the panel nor Settings shows.
    pub fn updated(&mut self, tx: &mut Tx) {
        self.restart(Restart::Update, tx);
    }

    /// Settings' explicit relaunch, installing a staged update first when present.
    pub fn relaunch(&mut self, tx: &mut Tx) {
        self.restart(Restart::Requested, tx);
    }

    pub fn resume_restart(&mut self, tx: &mut Tx) {
        if let Some(request) = self.relaunch.take() {
            self.restart(request, tx);
        }
    }

    /// The decision is made here, on the UI thread, so it sees the windows as they are now.
    /// Automatic updates wait so a restart does not interrupt work in a visible window.
    fn restart(&mut self, request: Restart, tx: &mut Tx) {
        if request == Restart::Update
            && (self.me.is_visible()
                || self.prefs.is_visible()
                || self.snapshots.is_visible()
                || self.script_output.is_visible())
        {
            self.relaunch = Some(request);
        } else if request.waiting_for_work(&self.running, self.runner.activity()) {
            self.relaunch_when_idle(request);
        } else {
            match self.updater.install_staged() {
                Ok(installed) => {
                    if installed || request == Restart::Requested {
                        if request == Restart::Requested {
                            self.stop_child_processes(tx);
                        }
                        tx.relaunch();
                    }
                }
                Err(error) => {
                    crate::log::warn(&error);
                    if request == Restart::Requested {
                        self.show_notice(error, tx);
                    }
                }
            }
        }
    }

    /// Ask again once every action already handed off has been carried out, so exiting
    /// doesn't cut short the app or file the user just opened. The panel may have been
    /// summoned meanwhile, so automatic updates check visibility again.
    fn relaunch_when_idle(&self, request: Restart) {
        let running = self.running.clone();
        let scripts = self.runner.activity().clone();
        let me = self.me.clone();
        std::thread::spawn(move || {
            while request.waiting_for_work(&running, &scripts) {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            me.update(move |s, tx| s.restart(request, tx));
        });
    }

    pub fn stop_child_processes(&mut self, _: &mut Tx) {
        self.runner.kill_all_now();
    }

    pub(super) fn quit(&mut self, tx: &mut Tx) {
        self.updater.install_on_quit();
        tx.quit();
    }

    pub fn set_panel_window(&mut self, window: rune::WindowHandle) {
        self.panel_window = Some(window);
    }

    #[cfg(target_os = "macos")]
    pub fn focus_changed(&mut self, focused: bool, tx: &mut Tx) {
        crate::trace::focus(focused);
        if !focused {
            self.dismiss_if_unfocused(tx);
        }
    }

    /// A focus loss hides the panel from `poll` once it has lasted `FOCUS_SETTLE` without
    /// focus coming back.
    #[cfg(target_os = "linux")]
    pub fn focus_changed(&mut self, focused: bool, tx: &mut Tx) {
        crate::trace::focus(focused);
        if focused {
            self.focus_wanted_until = None;
        }
        self.focus_lost_until = (!focused).then(|| tx.clock() + FOCUS_SETTLE);
        if let Some(due) = self.focus_lost_until {
            tx.wake_at(due);
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn poll_focus(&mut self, clock: f64, tx: &mut Tx) {
        if let Some(due) = self.focus_lost_until {
            if clock >= due {
                self.focus_lost_until = None;
                self.dismiss_if_unfocused(tx);
            } else {
                tx.wake_at(due);
            }
        }
        if let Some(due) = self.focus_wanted_until {
            if clock >= due {
                self.focus_wanted_until = None;
                if self.shown {
                    crate::trace::panel("refocus", self.me.is_visible(), self.shown);
                    self.me.focus();
                }
            } else {
                tx.wake_at(due);
            }
        }
    }

    pub(super) fn dismiss_if_unfocused(&mut self, tx: &mut Tx) {
        let auxiliary_owns_focus = self.sharing.is_active()
            || self
                .quick_look
                .borrow()
                .as_ref()
                .is_some_and(Previewer::panel_is_key);
        if self.shown && !auxiliary_owns_focus && !self.panel_is_key() {
            self.dismiss(tx);
        }
    }

    fn panel_is_key(&self) -> bool {
        self.panel_window.as_ref().is_some_and(|window| {
            window
                .with_raw_handle(|handle| match handle.as_raw() {
                    // SAFETY: Rune lends this live view only for this main-thread callback.
                    RawWindowHandle::AppKit(appkit) => unsafe {
                        crate::platform::window::is_key_window(appkit.ns_view)
                    },
                    _ => false,
                })
                .unwrap_or(false)
        })
    }

    #[cfg(target_os = "macos")]
    pub fn preview_panel_resigned(&mut self, tx: &mut Tx) {
        if !self.shown {
            return;
        }
        let keep = self
            .quick_look
            .borrow()
            .as_ref()
            .is_some_and(|preview| preview.finish_focus_transfer());
        if !keep {
            self.dismiss_if_unfocused(tx);
        }
    }
}

#[cfg(target_os = "macos")]
fn panel_offsets(frame: NSRect, visible: NSRect, width: u16, position: Position) -> (f32, f32) {
    let inset = visible.origin.x - frame.origin.x + ((visible.size.width - f64::from(width)) / 2.0).max(0.0);
    let top = frame.max().y - visible.max().y;
    // Use the query field's height so results can grow below it.
    let room = (visible.size.height - f64::from(crate::app::HEADER_HEIGHT)).max(0.0);
    let offset = match position {
        Position::Center => room / 2.0,
        Position::Top => f64::from(TOP_MARGIN).min(room),
    };
    (inset as f32, (top + offset) as f32)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use objc2_foundation::{NSPoint, NSSize};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    #[test]
    fn places_the_query_inside_the_selected_screens_visible_frame() {
        let frame = rect(100.0, -900.0, 1440.0, 900.0);
        let visible = rect(160.0, -850.0, 1380.0, 820.0);
        assert_eq!(
            panel_offsets(frame, visible, 680, Position::Center),
            (410.0, 408.0)
        );
        assert_eq!(panel_offsets(frame, visible, 680, Position::Top), (410.0, 210.0));
        assert_eq!(
            panel_offsets(frame, visible, 400, Position::Center),
            (550.0, 408.0)
        );
    }

    #[test]
    fn top_spacing_cannot_push_the_query_below_the_visible_area() {
        let frame = rect(-800.0, 200.0, 800.0, 150.0);
        let visible = rect(-800.0, 200.0, 800.0, 120.0);
        assert_eq!(panel_offsets(frame, visible, 680, Position::Top), (60.0, 86.0));
    }
}
