//! Dupin: a resident launcher panel. ⌘Space (by default) shows it; type to find an app, or
//! press Space first to search files with Spotlight.

/// A `&'static $type` the launcher keeps one of, made by `$init` on first use. Under test there
/// is one per thread instead: libtest runs each test on its own thread, and each test stands for
/// a launcher of its own, which mustn't see what another test set or wait behind its work.
macro_rules! launcher_wide {
    ($type:ty, $init:expr) => {{
        #[cfg(not(test))]
        {
            static VALUE: std::sync::OnceLock<$type> = std::sync::OnceLock::new();
            VALUE.get_or_init(|| $init)
        }
        #[cfg(test)]
        {
            thread_local! {
                static VALUE: &'static $type = Box::leak(Box::new($init));
            }
            VALUE.with(|value| *value)
        }
    }};
}

mod app;
mod backdrop;
mod calculator;
mod catalog_watch;
mod cli;
mod clipboard;
mod commands;
mod composition;
mod control;
mod extension_install;
mod extensions;
mod features;
mod hotkey;
mod http;
mod icons;
mod latency;
mod log;
mod mcp;
mod notices;
mod out;
mod prefs;
mod receivers;
mod recording;
mod scripts;
mod settings;
mod shader;
mod snapshots;
mod snippets;
mod store;
mod stored;
mod symbols;
mod sync;
mod sync_cli;
mod synced;
mod telemetry;
mod trace;
mod ui;
mod update;
mod view;
mod wall_clock;
mod watch_source;
mod web;

fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == dupin_extension_host::FLAG)
    {
        dupin_extension_host::serve();
    }
    if cli::run() {
        return;
    }
    app::run();
}
