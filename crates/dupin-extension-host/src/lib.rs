//! Compiles and runs JavaScript extensions outside Dupin, in a helper process that confines
//! itself with the platform sandbox before it touches any guest code. QuickJS-ng's parser runs on
//! untrusted source and cannot be interrupted, its interpreter cannot interrupt every native
//! operation, and its heap limit doesn't bound the process, so the boundary Dupin relies on is
//! one it can kill: the host stops the helper and starts another on the next call. Dupin keeps
//! only the bytecode a helper returns, as opaque bytes it sends back with each call. Each
//! package compiles in a helper started for it alone, so a helper one package compromised can't
//! plant bytecode for another that outlives the restart.

mod helper;
mod host;
mod sandbox;
mod wire;

pub use dupin_js_runtime::ActionEffect;
pub use helper::serve;
pub use host::{Compiled, Host, Launcher, NoServices, Services, Trust};

/// The argument that starts Dupin's executable as the extension host.
pub const FLAG: &str = "--extension-host";

/// Whether this process already runs inside a sandbox, such as the Nix build sandbox. The
/// extension host can't confine itself there, so it refuses to start.
pub fn is_sandboxed() -> bool {
    sandbox::inherited()
}

/// Whether the extension host has a sandbox to confine itself with on this platform.
pub const CAN_CONFINE: bool = cfg!(target_os = "macos");

/// Returns from the test it is in when this process is already sandboxed, where the extension
/// host can't confine itself, such as inside the Nix build sandbox, or when the platform has no
/// sandbox for it at all. A run with `DUPIN_REQUIRE_EXTENSION_HOST` set exists to exercise the
/// host, so inside a sandbox it fails instead.
#[cfg(any(test, feature = "testing"))]
#[macro_export]
macro_rules! skip_inside_a_sandbox {
    () => {{
        fn here() {}
        let test = ::std::any::type_name_of_val(&here).trim_end_matches("::here");
        if !$crate::CAN_CONFINE {
            let _ = ::std::io::Write::write_all(
                &mut ::std::io::stderr(),
                format!("skipped {test}: the extension host has no sandbox on this platform\n").as_bytes(),
            );
            return;
        }
        if $crate::is_sandboxed() {
            assert!(
                ::std::env::var_os("DUPIN_REQUIRE_EXTENSION_HOST").is_none(),
                "{test} must start the extension host, but this process is already sandboxed"
            );
            // Past the harness's output capture, so the skip shows in a passing run.
            let _ = ::std::io::Write::write_all(
                &mut ::std::io::stderr(),
                format!("skipped {test}: already inside a sandbox, where the extension host can't start\n")
                    .as_bytes(),
            );
            return;
        }
    }};
}

/// The helper refuses to run guests where there is no sandbox, so these run on macOS only.
#[cfg(all(test, target_os = "macos"))]
mod tests;
