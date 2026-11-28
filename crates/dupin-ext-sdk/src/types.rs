use std::path::Path;

use dupin_js_runtime::Diagnostic;

use crate::DECLARATIONS;
use crate::manifest::read_package;

/// Where `dupin ext new` puts [`DECLARATIONS`] in a package, and where `dupin ext types` keeps
/// them current.
pub const DECLARATIONS_FILE: &str = "dupin.d.ts";

/// How a package's `dupin.d.ts` compares with the one this Dupin ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Types {
    /// The same text as this Dupin's.
    Current,
    /// The package has none.
    Missing,
    /// Different text, describing the API version it declares, if it declares one.
    Different(Option<u32>),
}

impl Types {
    /// Whether the package's declarations may lack what this Dupin's describe: they differ, and
    /// don't declare a newer API than this Dupin's. `apiVersion` stays put while the API grows,
    /// so a copy of the same version can still be missing parts of it.
    pub fn is_outdated(self) -> bool {
        matches!(self, Self::Different(version) if version.is_none_or(|version| version <= api_version()))
    }
}

/// The API version this Dupin's declarations describe, which the host's `dupin` module exports.
pub fn api_version() -> u32 {
    declared_api_version(DECLARATIONS).expect("sdk/dupin.d.ts declares apiVersion")
}

/// Compares the package's `dupin.d.ts` with this Dupin's.
pub fn check_types(dir: &Path) -> Result<Types, Diagnostic> {
    read_package(dir)?;
    match std::fs::read_to_string(dir.join(DECLARATIONS_FILE)) {
        Ok(text) if text == DECLARATIONS => Ok(Types::Current),
        Ok(text) => Ok(Types::Different(declared_api_version(&text))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Types::Missing),
        Err(error) => {
            Err(Diagnostic::new("unreadable", format!("Couldn't read it: {error}")).file(DECLARATIONS_FILE))
        }
    }
}

/// Writes this Dupin's declarations to the package's `dupin.d.ts`, returning how the file
/// compared before. The new file replaces the old entry by a rename, so a symlink is replaced
/// rather than followed out of the package, and a failed write leaves the old file whole.
pub fn update_types(dir: &Path) -> Result<Types, Diagnostic> {
    let before = check_types(dir)?;
    if before != Types::Current {
        let staged = dir.join(format!(".{DECLARATIONS_FILE}.tmp"));
        let unwritable = |error: std::io::Error| {
            Diagnostic::new("unwritable", format!("Couldn't write it: {error}")).file(DECLARATIONS_FILE)
        };
        std::fs::write(&staged, DECLARATIONS).map_err(unwritable)?;
        std::fs::rename(&staged, dir.join(DECLARATIONS_FILE)).map_err(|error| {
            let _ = std::fs::remove_file(&staged);
            unwritable(error)
        })?;
    }
    Ok(before)
}

fn declared_api_version(text: &str) -> Option<u32> {
    let rest = &text[text.find("export const apiVersion:")? + "export const apiVersion:".len()..];
    let digits = rest.trim_start();
    let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());
    digits[..end].parse().ok()
}
