//! Runs JavaScript extensions outside Dupin, in a helper process that confines itself with the
//! platform sandbox before it runs any guest code. QuickJS-ng cannot interrupt every native
//! operation, and its heap limit doesn't bound the process, so the boundary Dupin relies on is
//! one it can kill: the host stops the helper and starts another on the next call.

mod helper;
mod host;
mod sandbox;
mod wire;

pub use helper::serve;
pub use host::{Host, Launcher};

/// The argument that starts Dupin's executable as the extension host.
pub const FLAG: &str = "--extension-host";

/// The helper refuses to run guests where there is no sandbox, so these run on macOS only.
#[cfg(all(test, target_os = "macos"))]
mod tests;
