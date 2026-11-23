//! Dupin: a resident launcher panel. ⌘Space (by default) shows it; type to find an app, or
//! press Space first to search files with Spotlight.

mod backdrop;
mod commands;
mod hotkey;
mod icons;
mod log;
mod prefs;
mod settings;
mod shader;
mod snapshots;
mod store;
mod trace;
mod ui;
mod update;
mod view;
mod web;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rune::{App, AppSpec, WindowOptions, WindowSpec};

/// The panel's width; its height follows its content.
pub const PANEL_WIDTH: f32 = 680.0;
/// The panel with nothing below the field: the window's first size, before it fits its content.
pub const HEADER_HEIGHT: f32 = view::HEADER_H;
/// San Francisco's family, as its file names it (`dupin_mac::font::system_ui_font`).
const FAMILIES: (&str, &str) = ("System Font", "ui-monospace");

fn main() {
    rune::prewarm_gpu();
    let (config, config_problem) = settings::load();
    let history = store::load();
    // The system font instead of Rune's embedded Inter, which the build leaves out.
    let fonts: &'static [&'static [u8]] = match dupin_mac::font::system_ui_font() {
        Some(font) => Box::leak(Box::new([font])),
        None => {
            log::warn("couldn't map the system font; text falls back through CoreText");
            &[]
        }
    };

    let spec = AppSpec {
        setup: Some(ui::Ui::setup),
        poll: Some(ui::Ui::poll),
        on_key: Some(ui::Ui::unfocused_key),
        fonts,
        families: FAMILIES,
        ..AppSpec::new(
            "Dupin",
            (PANEL_WIDTH, HEADER_HEIGHT),
            || unreachable!(),
            view::view,
        )
    };
    // Blend in sRGB as Core Animation does, so the translucent frost over the glass reads as a
    // native layer would.
    let app = App::new(spec)
        .windowless()
        .accessory()
        .fit_content_height()
        .srgb_blend(true)
        .compositor(backdrop::Backdrop::default());
    let handle = app.handle();
    let binding = Arc::new(hotkey::Binding::new(handle.clone()));

    // The settings window: an ordinary titled window, made when ⌘, first opens it.
    let prefs = App::new(AppSpec {
        fonts,
        families: FAMILIES,
        ..AppSpec::new("Dupin Settings", prefs::SIZE, || unreachable!(), prefs::view)
    })
    .windowless()
    .on_window(|_| trace::settings("created"))
    .on_ready(|_| trace::settings("ready"))
    .on_focus(|_, focused, _| {
        trace::settings(if focused { "focused" } else { "unfocused" });
    })
    .on_close(prefs::Prefs::closed);
    let prefs_handle = prefs.handle();
    let snapshots = App::new(AppSpec {
        fonts,
        families: FAMILIES,
        on_key: Some(snapshots::Snapshots::key),
        ..AppSpec::new(
            "Dupin Snapshots",
            (900.0, 600.0),
            || unreachable!(),
            snapshots::view,
        )
    })
    .windowless()
    .on_close(snapshots::Snapshots::closed);
    let snapshots_handle = snapshots.handle();
    let snapshots = snapshots.init({
        let panel = handle.clone();
        move || snapshots::Snapshots::new(panel)
    });
    let settings = Arc::new(settings::Service::new(settings::path(), {
        let panel = handle.clone();
        move |update| panel.update(move |s, tx| s.reload_config(update, tx))
    }));
    let updater = update::Updater::start(config.auto_update, handle.clone(), prefs_handle.clone());
    let current_config = Rc::new(RefCell::new(config.clone()));
    let prefs = prefs.init({
        let (config, binding, panel, updater, settings) = (
            current_config.clone(),
            binding.clone(),
            handle.clone(),
            updater.clone(),
            settings.clone(),
        );
        move || prefs::Prefs::new(config, binding, panel, updater, settings)
    });

    let hotkey = config.hotkey.clone();
    let mut registered = false;
    let app = app
        .init({
            let handle = handle.clone();
            let configuration = ui::Configuration {
                values: current_config,
                hotkey: binding.clone(),
                files: settings.clone(),
            };
            move || {
                ui::Ui::new(
                    handle,
                    prefs_handle,
                    snapshots_handle,
                    updater,
                    configuration,
                    history,
                    config_problem,
                )
            }
        })
        .on_ready(move |window| {
            appear_instantly(window);
            // Once per process: a launcher window is made once and kept, but be explicit.
            if !std::mem::replace(&mut registered, true) {
                binding.register_at_launch(&hotkey);
                settings.activate();
            }
        })
        .on_frame_encoded(trace::frame)
        .on_close(ui::Ui::closed)
        .on_focus(|s, focused, tx| {
            if !focused {
                s.dismiss(tx);
            }
        });

    rune::run_windows(move |_| {
        // AppKit rounds the panel and its shadow; the view also clips, for headless renders.
        let options = WindowOptions::launcher_panel().macos(|native| native.corner_radius(view::RADIUS));
        let panel: rune::AnyWindowSpec = WindowSpec::from_app("panel", app)
            .configured_options(options)
            .into();
        let prefs: rune::AnyWindowSpec = WindowSpec::from_app("prefs", prefs)
            .options(WindowOptions {
                resizable: false,
                ..Default::default()
            })
            .into();
        let snapshots: rune::AnyWindowSpec = WindowSpec::from_app("snapshots", snapshots).into();
        [panel, prefs, snapshots]
    });
}

/// Turn off AppKit's ordering animation on the panel. `on_ready` runs once the panel exists
/// (Rune moves the content into its own NSPanel at creation), on the main thread.
fn appear_instantly(window: &rune::WindowHandle) {
    // A headless `--realtime` run calls `on_ready` too, with no native window to change.
    if rune::tooling::headless_run().is_some() {
        return;
    }
    use rune::raw_window_handle::RawWindowHandle;
    let done = window.with_raw_handle(|handle| match handle.as_raw() {
        // SAFETY: The handle is borrowed inside `with_raw_handle`, so its view is alive.
        RawWindowHandle::AppKit(appkit) => unsafe { dupin_mac::window::appear_instantly(appkit.ns_view) },
        _ => false,
    });
    if done != Ok(true) {
        log::warn(&format!("couldn't turn off the panel's show animation: {done:?}"));
    }
}
