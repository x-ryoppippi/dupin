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
    /// Whether the package's declarations describe an older API than this Dupin's, so the
    /// author's editor is missing what the host now offers.
    pub fn is_older(self) -> bool {
        matches!(self, Self::Different(version) if version.is_none_or(|version| version < api_version()))
    }
}

/// The API version this Dupin's declarations describe, which the host's `dupin` module exports.
pub fn api_version() -> u32 {
    declared_api_version(DECLARATIONS).expect("sdk/dupin.d.ts declares apiVersion")
}

/// Compares the package's `dupin.d.ts` with this Dupin's.
pub fn check_types(dir: &Path) -> Types {
    match std::fs::read_to_string(dir.join(DECLARATIONS_FILE)) {
        Ok(text) if text == DECLARATIONS => Types::Current,
        Ok(text) => Types::Different(declared_api_version(&text)),
        Err(_) => Types::Missing,
    }
}

/// Writes this Dupin's declarations to the package's `dupin.d.ts`, returning how the file
/// compared before.
pub fn update_types(dir: &Path) -> Result<Types, Diagnostic> {
    read_package(dir)?;
    let before = check_types(dir);
    if before != Types::Current {
        let path = dir.join(DECLARATIONS_FILE);
        std::fs::write(&path, DECLARATIONS).map_err(|error| {
            Diagnostic::new("unwritable", format!("Couldn't write it: {error}"))
                .file(path.display().to_string())
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
