//! The one place the CLI writes to stdout.
//!
//! Rust ignores SIGPIPE, so a reader that closes the pipe early (`dupin ... --json | head`)
//! makes `println!` panic on EPIPE. A closed reader is not a failure, so these macros drop the
//! write error, exactly as the plain `--licenses` path does.

use std::io::Write;

pub(crate) fn write(args: std::fmt::Arguments<'_>) {
    let _ = std::io::stdout().lock().write_fmt(args);
}

pub(crate) fn write_line(args: std::fmt::Arguments<'_>) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_fmt(args).and_then(|()| out.write_all(b"\n"));
}

macro_rules! out {
    ($($arg:tt)*) => { $crate::out::write(format_args!($($arg)*)) };
}

macro_rules! outln {
    () => { $crate::out::write(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::out::write_line(format_args!($($arg)*)) };
}

pub(crate) use {out, outln};
