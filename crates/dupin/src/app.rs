use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rune::{App, AppSpec, WindowOptions, WindowSpec};

use crate::{
    backdrop, control, hotkey, log, prefs, scripts, settings, snapshots, store, trace, ui, update, view,
};

/// The panel with nothing below the field: the window's first size, before it fits its content.
pub(crate) const HEADER_HEIGHT: f32 = view::HEADER_H;
/// San Francisco's family, as its file names it (`dupin_mac::font::system_ui_font`).
const FAMILIES: (&str, &str) = ("System Font", "ui-monospace");

pub(crate) fn run() {
    let headless = rune::tooling::is_headless();
    if !headless {
        std::thread::spawn(|| {
            if let Err(error) = dupin_mac::file_ops::FileTransfers::default().recover_abandoned_copies() {
                log::warn(&format!("couldn't clean up abandoned copies: {error}"));
            }
        });
        rune::prewarm_gpu();
    }
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
            (config.theme.panel_width as f32, HEADER_HEIGHT),
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
    let _control = if headless {
        None
    } else {
        let panel = handle.clone();
        match control::Server::start(move |call| panel.update(move |s, tx| s.control(call, tx))) {
            Ok(server) => Some(server),
            Err(error) => {
                log::warn(&format!("local control unavailable: {error}"));
                None
            }
        }
    };
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
    .on_focus(|s, focused, _| {
        if focused {
            s.refresh_login_item();
            s.refresh_permissions();
            s.refresh_hotkey();
        }
        trace::settings(if focused { "focused" } else { "unfocused" });
    })
    .on_close(prefs::Prefs::closed);
    let prefs_handle = prefs.handle();
    let calendar_access = dupin_mac::privacy::CalendarAccess::new({
        let panel = handle.clone();
        let prefs = prefs_handle.clone();
        move || {
            panel.update(ui::Ui::calendar_access_changed);
            prefs.update(|s, _| s.refresh_permissions());
        }
    });
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
    let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let snapshots = snapshots.init({
        let panel = handle.clone();
        let me = snapshots_handle.clone();
        let running = running.clone();
        move || snapshots::Snapshots::new(panel, me, running)
    });
    let scripts = App::new(AppSpec {
        fonts,
        families: FAMILIES,
        on_key: Some(scripts::output::Output::key),
        ..AppSpec::new(
            "Dupin Script Output",
            (900.0, 600.0),
            || unreachable!(),
            scripts::output::view,
        )
    })
    .windowless()
    .srgb_blend(true)
    .on_close(scripts::output::Output::closed);
    let scripts_handle = scripts.handle();
    let scripts = scripts.init({
        let panel = handle.clone();
        move || scripts::output::Output::new(panel)
    });
    let settings = Arc::new(settings::Service::new(settings::path(), {
        let panel = handle.clone();
        move |update| panel.update(move |s, tx| s.reload_config(update, tx))
    }));
    let updater = update::Updater::start(config.auto_update, handle.clone(), prefs_handle.clone());
    let current_config = Rc::new(RefCell::new(config.clone()));
    let welcome = prefs::welcome::first_run();
    let prefs = prefs.init({
        let calendar_access = calendar_access.clone();
        let (config, binding, panel, updater, settings) = (
            current_config.clone(),
            binding.clone(),
            handle.clone(),
            updater.clone(),
            settings.clone(),
        );
        move || {
            prefs::Prefs::new(
                config,
                binding,
                panel,
                updater,
                settings,
                welcome,
                calendar_access,
            )
        }
    });
    if welcome {
        prefs_handle.open();
    }

    let settings_window = prefs_handle.clone();
    let radius = config.theme.corner_radius as f32;
    let mut registered = false;
    let quick_look = Rc::new(RefCell::new(None));
    let app = app
        .init({
            let handle = handle.clone();
            let quick_look = quick_look.clone();
            let configuration = ui::Configuration {
                values: current_config,
                hotkey: binding.clone(),
                files: settings.clone(),
            };
            move || {
                let mut ui = ui::Ui::new(
                    ui::Windows {
                        panel: handle,
                        prefs: prefs_handle,
                        snapshots: snapshots_handle,
                        scripts: scripts_handle,
                    },
                    updater,
                    configuration,
                    history,
                    calendar_access,
                    quick_look,
                    running,
                );
                ui.config_problem = config_problem;
                ui
            }
        })
        .on_ready(move |window| {
            appear_instantly(window);
            let panel = window.clone();
            handle.update(move |ui, _| ui.set_panel_window(panel));
            install_quick_look(window, &handle, &quick_look);
            // Once per process: a launcher window is made once and kept, but be explicit.
            if !std::mem::replace(&mut registered, true) {
                binding.register_at_launch(&config, !welcome);
                settings_window.update(|s, _| s.reflect_launch_hotkey());
                settings.activate();
            }
        })
        .on_frame_encoded(trace::frame)
        .on_quit(ui::Ui::stop_child_processes)
        .on_close(ui::Ui::closed)
        .on_focus(ui::Ui::focus_changed);

    rune::run_windows(move |_| {
        // AppKit rounds the panel and its shadow; the view also clips, for headless renders.
        let options = WindowOptions::launcher_panel().macos(|native| native.corner_radius(radius));
        let panel: rune::AnyWindowSpec = WindowSpec::from_app("panel", app)
            .configured_options(options)
            .into();
        let prefs: rune::AnyWindowSpec = WindowSpec::from_app("prefs", prefs)
            .options(WindowOptions {
                resizable: false,
                ..Default::default()
            })
            .into();
        let snapshots: rune::AnyWindowSpec = WindowSpec::from_app("snapshots", snapshots)
            .options(WindowOptions {
                min_size: Some((720.0, 420.0)),
                ..Default::default()
            })
            .into();
        let scripts: rune::AnyWindowSpec = WindowSpec::from_app("scripts", scripts)
            .options(WindowOptions {
                min_size: Some((720.0, 420.0)),
                ..Default::default()
            })
            .into();
        [panel, prefs, snapshots, scripts]
    });
}

fn install_quick_look(
    window: &rune::WindowHandle,
    app: &rune::AppHandle<ui::Ui>,
    slot: &Rc<RefCell<Option<dupin_mac::quick_look::Previewer>>>,
) {
    if rune::tooling::headless_run().is_some() {
        return;
    }
    use rune::raw_window_handle::RawWindowHandle;
    let app = app.clone();
    let attached = window.with_raw_handle(|handle| match handle.as_raw() {
        RawWindowHandle::AppKit(appkit) => {
            // SAFETY: Rune lends this NSView only for the duration of `with_raw_handle`; attach
            // retains it before adding the responder proxy.
            unsafe {
                dupin_mac::quick_look::Previewer::attach(appkit.ns_view, move || {
                    app.update(|state, tx| state.preview_panel_resigned(tx));
                })
            }
        }
        _ => None,
    });
    match attached {
        Ok(Some(previewer)) => *slot.borrow_mut() = Some(previewer),
        Ok(None) => crate::log::warn("couldn't attach the Quick Look responder"),
        Err(error) => crate::log::warn(&format!("couldn't access the panel's native view: {error}")),
    }
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
