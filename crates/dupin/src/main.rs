//! Dupin: a resident launcher panel. ⌘Space (by default) shows it; type to find an app, or
//! press Space first to search files with Spotlight.

mod app;
mod backdrop;
mod calculator;
mod catalog_watch;
mod cli;
mod clipboard;
mod commands;
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
mod prefs;
mod receivers;
mod scripts;
mod settings;
mod shader;
mod snapshots;
mod store;
mod trace;
mod ui;
mod update;
mod view;
mod wall_clock;
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
