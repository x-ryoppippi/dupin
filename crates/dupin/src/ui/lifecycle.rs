use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dupin_core::{DisplaySelection, InputSource, Position};
use dupin_mac::{accessibility, quick_look::Previewer, window::DisplayNotice};
use objc2_foundation::NSRect;
use rune::raw_window_handle::RawWindowHandle;
use rune::{Align, DisplayTarget, OverlayPlacement, ScreenEdge, Tx};

use super::{PendingPaste, Ui, selection::SelectionRequest};

// Match Spotlight's offset for the top position, and mirror it for the bottom.
const EDGE_MARGIN: f32 = 180.0;

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

fn toggle_should_dismiss(visible: bool, shown: bool) -> bool {
    visible && shown
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

#[test]
fn toggle_reopens_when_window_visibility_and_shown_state_disagree() {
    assert!(toggle_should_dismiss(true, true));
    assert!(!toggle_should_dismiss(true, false));
    assert!(!toggle_should_dismiss(false, true));
    assert!(!toggle_should_dismiss(false, false));
}

impl Ui {
    /// Whether the panel is on screen: shown, or drawn by a headless run, which never summons it.
    pub(super) fn on_screen(&self) -> bool {
        self.shown || rune::tooling::is_headless()
    }

    /// The hotkey: show the panel, or hide it when it already shows.
    pub(super) fn toggle(&mut self, tx: &mut Tx) {
        if self.picker.is_some() {
            self.finish_pick(None, tx);
            return;
        }
        let visible = self.me.is_visible();
        crate::trace::panel("toggle", visible, self.shown);
        // Rune can still report the window visible after hide sets `shown` to false.
        // In that transition, dismiss would return without doing anything.
        if toggle_should_dismiss(visible, self.shown) {
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
            if self.notice().is_none()
                && let Some(notice) =
                    crate::telemetry::get().and_then(crate::telemetry::Telemetry::take_notice)
            {
                // Hiding the panel clears it.
                self.notices.persistent(notice.to_owned(), None);
            }
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
        let edge = panel_edge(self.position);
        let geometry = (!rune::tooling::is_headless())
            .then(|| dupin_mac::window::display_geometry(self.display))
            .flatten();
        let screen = match &geometry {
            Some(geometry) => Some(PanelScreen::new(geometry)),
            None if rune::tooling::is_headless() => PanelScreen::simulated(),
            None => None,
        };
        if screen != self.screen {
            self.screen = screen;
            self.prefs
                .update(move |prefs, _| prefs.reflect_panel_screen(screen));
        }
        self.fit_list();
        let (display, align, inset, margin) = if let Some(screen) = geometry {
            self.display_notice = screen.notice;
            let place = panel_offsets(
                screen.frame,
                screen.visible_frame,
                self.theme.panel_width,
                self.position,
            );
            self.pin_bottom_edge(
                (edge == ScreenEdge::Bottom).then(|| screen.frame.origin.y + f64::from(place.margin)),
            );
            (
                DisplayTarget::Native(rune::NativeDisplayId::MacOS(screen.native_id)),
                Align::Start,
                place.inset,
                place.margin,
            )
        } else {
            let display = match self.display {
                DisplaySelection::Mouse => DisplayTarget::UnderPointer,
                DisplaySelection::Main | DisplaySelection::Active => DisplayTarget::Primary,
            };
            (display, Align::Center, 0.0, EDGE_MARGIN)
        };
        self.me.place(
            display,
            OverlayPlacement::Edge {
                edge,
                margin,
                align,
                inset,
            },
        );
    }

    /// Size the result list for the display the panel last opened on and the configured rows.
    pub(super) fn fit_list(&mut self) {
        let room = self
            .screen
            .map(|screen| screen.list_room(self.theme.panel_width, self.position));
        self.list_budget = crate::view::list_fit(room, self.rows, f32::from(self.theme.row_height)).budget;
    }

    /// Keep the panel's bottom edge at `bottom` as it fits its content, or let it go.
    fn pin_bottom_edge(&self, bottom: Option<f64>) {
        let Some(window) = &self.panel_window else {
            return;
        };
        let pinned = window.with_raw_handle(|handle| match handle.as_raw() {
            RawWindowHandle::AppKit(appkit) => {
                // SAFETY: Rune lends this live view only for this main-thread callback.
                unsafe { dupin_mac::window::pin_bottom_edge(appkit.ns_view, bottom) };
                true
            }
            _ => false,
        });
        if bottom.is_some() && !pinned.unwrap_or(false) {
            crate::log::warn("couldn't hold the panel's bottom edge; it grows downwards");
        }
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

    fn force_input_source(&mut self) {
        use dupin_mac::input_source::{Target, select};
        if self.restore_input.is_some() {
            return;
        }
        let target = match &self.input_source {
            InputSource::Keep => return,
            InputSource::Ascii => Target::Ascii,
            InputSource::Id(id) => Target::Id(id.clone()),
        };
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
                let settings = dupin_mac::workspace::Action::OpenUrl(accessibility::SETTINGS_URL.into());
                if let Err(error) = dupin_mac::workspace::run(&settings) {
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
            self.prefs.update(|prefs, _| prefs.open_snapshots());
        }
        if show_settings {
            crate::trace::settings("panel closed");
            self.prefs.update(|prefs, _| prefs.open_settings());
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

    pub(super) fn run_app_command(&mut self, command: dupin_core::AppCommand, tx: &mut Tx) {
        use crate::update::Status;
        if command == dupin_core::AppCommand::Relaunch {
            return self.relaunch(tx);
        }
        let status = self.updater.status();
        if matches!(status, Status::Ready(_)) {
            return self.relaunch(tx);
        }
        let version = self.updater.version().to_owned();
        if !status.can_check() {
            return self.show_notice(status.outcome(&version), tx);
        }
        // The check ends on the updater's own thread; wait for it off this one.
        let updater = self.updater.clone();
        let me = self.me.clone();
        std::thread::spawn(move || {
            let outcome = match updater.check_and_wait(std::time::Duration::from_secs(10 * 60)) {
                Some(Status::Ready(_)) => return me.update(|ui, tx| ui.relaunch(tx)),
                Some(status) => status.outcome(&version),
                None => "Dupin couldn't finish checking for updates.".into(),
            };
            me.update(move |ui, tx| ui.show_notice(outcome, tx));
        });
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
            && (self.me.is_visible() || self.prefs.is_visible() || self.script_output.is_visible())
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

    pub fn focus_changed(&mut self, focused: bool, tx: &mut Tx) {
        if !focused {
            self.dismiss_if_unfocused(tx);
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
                        dupin_mac::window::is_key_window(appkit.ns_view)
                    },
                    _ => false,
                })
                .unwrap_or(false)
        })
    }

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

/// The display the panel opens on, as much of it as sizing the panel's list needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelScreen {
    frame: NSRect,
    visible: NSRect,
}

impl PanelScreen {
    fn new(geometry: &dupin_mac::window::DisplayGeometry) -> Self {
        Self {
            frame: geometry.frame,
            visible: geometry.visible_frame,
        }
    }

    /// The display the panel would open on now, for `selection`; in a headless run, the one
    /// `DUPIN_HEADLESS_DISPLAY` simulates, if any.
    pub fn current(selection: DisplaySelection) -> Option<Self> {
        if rune::tooling::is_headless() {
            return Self::simulated();
        }
        dupin_mac::window::display_geometry(selection).map(|geometry| Self::new(&geometry))
    }

    /// A headless run's display, whose usable area `DUPIN_HEADLESS_DISPLAY` gives as
    /// `<width>x<height>` points, so the list is sized as on a real display of that size.
    fn simulated() -> Option<Self> {
        let value = std::env::var("DUPIN_HEADLESS_DISPLAY").ok()?;
        let screen = Self::parse(&value);
        if screen.is_none() {
            crate::log::warn(&format!(
                "DUPIN_HEADLESS_DISPLAY is {value:?}, not <width>x<height>; using the default list"
            ));
        }
        screen
    }

    /// A display whose whole frame is usable, `value` points as `<width>x<height>`.
    fn parse(value: &str) -> Option<Self> {
        let (w, h) = value.split_once('x')?;
        let (w, h) = (w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?);
        if !(w > 0.0 && h > 0.0) {
            return None;
        }
        let area = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(w, h),
        );
        Some(Self {
            frame: area,
            visible: area,
        })
    }

    /// The height the display leaves on the result list's side of the field.
    pub fn list_room(self, width: u16, position: Position) -> f32 {
        panel_offsets(self.frame, self.visible, width, position).list_room
    }
}

/// Where the panel goes on a display, in points from the display's frame.
#[derive(Debug, PartialEq)]
struct PanelOffsets {
    /// From the display's left edge to the panel's.
    inset: f32,
    /// From the display's edge the panel is placed against (`panel_edge`) to the panel's.
    margin: f32,
    /// The height the visible area leaves on the result list's side of the field.
    list_room: f32,
}

/// The display edge the panel keeps still as it fits its content: the field's side.
fn panel_edge(position: Position) -> ScreenEdge {
    if position.grows_up() {
        ScreenEdge::Bottom
    } else {
        ScreenEdge::Top
    }
}

fn panel_offsets(frame: NSRect, visible: NSRect, width: u16, position: Position) -> PanelOffsets {
    let inset = visible.origin.x - frame.origin.x + ((visible.size.width - f64::from(width)) / 2.0).max(0.0);
    // From the display's edge to the visible area's, on the side the panel is placed against.
    let outside = match panel_edge(position) {
        ScreenEdge::Top => frame.max().y - visible.max().y,
        ScreenEdge::Bottom => visible.origin.y - frame.origin.y,
    };
    // Use the query field's height so results can grow away from it.
    let room = (visible.size.height - f64::from(crate::app::HEADER_HEIGHT)).max(0.0);
    let offset = match position {
        Position::Center => room / 2.0,
        Position::Top | Position::Bottom => f64::from(EDGE_MARGIN).min(room),
    };
    PanelOffsets {
        inset: inset as f32,
        margin: (outside + offset) as f32,
        list_room: (room - offset) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::{NSPoint, NSSize};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    fn offsets(inset: f32, margin: f32, list_room: f32) -> PanelOffsets {
        PanelOffsets {
            inset,
            margin,
            list_room,
        }
    }

    #[test]
    fn places_the_query_inside_the_selected_screens_visible_frame() {
        let frame = rect(100.0, -900.0, 1440.0, 900.0);
        let visible = rect(160.0, -850.0, 1380.0, 820.0);
        assert_eq!(
            panel_offsets(frame, visible, 680, Position::Center),
            offsets(410.0, 408.0, 378.0)
        );
        assert_eq!(
            panel_offsets(frame, visible, 680, Position::Top),
            offsets(410.0, 210.0, 576.0)
        );
        assert_eq!(
            panel_offsets(frame, visible, 400, Position::Center),
            offsets(550.0, 408.0, 378.0)
        );
    }

    #[test]
    fn top_spacing_cannot_push_the_query_below_the_visible_area() {
        let frame = rect(-800.0, 200.0, 800.0, 150.0);
        let visible = rect(-800.0, 200.0, 800.0, 120.0);
        assert_eq!(
            panel_offsets(frame, visible, 680, Position::Top),
            offsets(60.0, 86.0, 0.0)
        );
    }

    #[test]
    fn a_simulated_display_sizes_the_list_as_a_real_one_would() {
        use dupin_core::Rows;
        let budget = |room| crate::view::list_fit(room, Rows::Auto, 52.0).budget;
        let real = rect(0.0, 0.0, 1728.0, 2560.0);
        let room = panel_offsets(real, real, 680, Position::Center).list_room;
        let simulated = PanelScreen::parse("1728x2560").unwrap();
        assert_eq!(simulated.list_room(680, Position::Center), room);
        assert_eq!(
            budget(Some(room)),
            832.0,
            "a 2560-point display reaches the list's maximum"
        );
        let short = PanelScreen::parse("1440x900").unwrap();
        assert_eq!(budget(Some(short.list_room(680, Position::Center))), budget(None));
        for bad in ["", "1440", "1440x", "x900", "0x900", "1440x-1", "wide x tall"] {
            assert_eq!(PanelScreen::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_bottom_position_mirrors_the_top_from_the_visible_frames_bottom() {
        // The Dock takes 50 points along the bottom, the menu bar 30 along the top.
        let frame = rect(100.0, -900.0, 1440.0, 900.0);
        let visible = rect(160.0, -850.0, 1380.0, 820.0);
        assert_eq!(panel_edge(Position::Bottom), ScreenEdge::Bottom);
        assert_eq!(panel_edge(Position::Top), ScreenEdge::Top);
        assert_eq!(panel_edge(Position::Center), ScreenEdge::Top);
        assert_eq!(
            panel_offsets(frame, visible, 680, Position::Bottom),
            offsets(410.0, 230.0, 576.0)
        );
        let short = rect(-800.0, 200.0, 800.0, 120.0);
        assert_eq!(
            panel_offsets(short, short, 680, Position::Bottom),
            offsets(60.0, 56.0, 0.0)
        );
    }

    #[test]
    fn a_taller_display_leaves_the_list_more_room() {
        let short = rect(0.0, 0.0, 1440.0, 900.0);
        let tall = rect(0.0, 0.0, 2560.0, 1440.0);
        for position in [Position::Center, Position::Top, Position::Bottom] {
            let short = panel_offsets(short, short, 680, position).list_room;
            let tall = panel_offsets(tall, tall, 680, position).list_room;
            assert!(tall > short, "{position:?}: {tall} <= {short}");
        }
    }
}
