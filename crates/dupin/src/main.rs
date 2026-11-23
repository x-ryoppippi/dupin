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
mod extensions;
mod hotkey;
mod http;
mod icons;
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
mod window_management;

fn main() {
    if cli::run() {
        return;
    }
    app::run();
}
