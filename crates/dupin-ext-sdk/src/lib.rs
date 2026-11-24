//! What an extension author runs on their own machine: `dupin ext new`, `dupin ext build` and
//! `dupin ext dev`. Nothing here runs inside the launcher; the launcher only ever sees the
//! JavaScript these commands produce. Whether that JavaScript loads is decided by the launcher's
//! own loader, which callers pass to `build` and `dev` as `validate`.

mod build;
pub mod dev;
mod manifest;
mod scaffold;

pub use build::{BuildOptions, Built, Bundler, BundlerKind, build};
pub use dupin_js_runtime::Diagnostic;
pub use manifest::PackageInfo;
pub use scaffold::{Template, scaffold};

/// The declarations `dupin ext new` copies into a package, for the API version the host's
/// `dupin` module reports.
pub(crate) const DECLARATIONS: &str = include_str!("../../../sdk/dupin.d.ts");

#[cfg(test)]
mod tests;
