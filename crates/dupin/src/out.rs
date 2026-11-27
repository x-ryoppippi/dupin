//! The one place the CLI writes to stdout.
//!
//! Rust ignores SIGPIPE, so a reader that closes the pipe early (`dupin ... --json | head`)
//! makes `println!` panic on EPIPE. A closed reader wants no more output, so Dupin stops there
//! and exits successfully; any other write error, such as a full disk behind a redirect, would
//! leave the output cut short, so it is reported and the command fails.

use std::io::{ErrorKind, Write};

pub(crate) fn write(args: std::fmt::Arguments<'_>) {
    finish(std::io::stdout().lock().write_fmt(args));
}

pub(crate) fn write_line(args: std::fmt::Arguments<'_>) {
    let mut out = std::io::stdout().lock();
    finish(out.write_fmt(args).and_then(|()| out.write_all(b"\n")));
}

fn finish(result: std::io::Result<()>) {
    match result {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::BrokenPipe => std::process::exit(0),
        Err(e) => {
            eprintln!("dupin: couldn't write to stdout: {e}");
            std::process::exit(1);
        }
    }
}

macro_rules! out {
    ($($arg:tt)*) => { $crate::out::write(format_args!($($arg)*)) };
}

macro_rules! outln {
    () => { $crate::out::write(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::out::write_line(format_args!($($arg)*)) };
}

pub(crate) use {out, outln};
