//! What an extension author runs on their own machine: `dupin ext new`, `dupin ext build` and
//! `dupin ext dev`. Nothing here runs inside the launcher; the launcher only ever sees the
//! JavaScript these commands produce.

mod build;
pub mod dev;
mod manifest;
mod scaffold;

use std::path::Path;
use std::time::{Duration, Instant};

pub use build::{BuildOptions, Built, Bundler, BundlerKind, build};
pub use dupin_js_runtime::Diagnostic;
pub use manifest::PackageInfo;
use manifest::read_package;
pub use scaffold::{Template, scaffold};

/// The declarations `dupin ext new` copies into a package, for the API version the host's
/// `dupin` module reports.
pub(crate) const DECLARATIONS: &str = include_str!("../../../sdk/dupin.d.ts");

const CHECK_DEADLINE: Duration = Duration::from_secs(2);

/// Reads the manifest and loads and links the entry as the launcher's JavaScript runtime does.
pub fn check(dir: &Path) -> Result<PackageInfo, Diagnostic> {
    let package = read_package(dir)?;
    let module = dupin_js_runtime::Package::load(dir, &package.main)?;
    module
        .check(dupin_js_runtime::Control {
            deadline: Instant::now() + CHECK_DEADLINE,
            is_cancelled: Box::new(|| false),
        })
        .map_err(|error| match error {
            dupin_js_runtime::RunError::Failed(diagnostic) => diagnostic,
            dupin_js_runtime::RunError::Cancelled => Diagnostic::new("cancelled", "the check was cancelled"),
        })?;
    Ok(package)
}

#[cfg(test)]
mod tests;
