//! The licences and copyright notices of the third-party code and data Dupin ships, which the
//! app build writes to `Contents/Resources/ThirdPartyNotices.txt` (packaging/notices.nu). A
//! build run from the source tree has none.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const FILE: &str = "ThirdPartyNotices.txt";

/// The running Dupin.app's notices, if it carries them.
pub fn path() -> Option<&'static Path> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let executable = std::env::current_exe().ok()?.canonicalize().ok()?;
        in_app(&executable).filter(|path| path.is_file())
    })
    .as_deref()
}

fn in_app(executable: &Path) -> Option<PathBuf> {
    crate::extensions::bundled::resources(executable).map(|resources| resources.join(FILE))
}

/// The notices' text, or why there is none.
pub fn read() -> Result<(PathBuf, String), String> {
    let path = path().ok_or(
        "this Dupin isn't running from Dupin.app, so it has no third-party notices; \
         `nix build .#dupin` writes them to Dupin.app/Contents/Resources/ThirdPartyNotices.txt",
    )?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    Ok((path.to_owned(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_keeps_its_notices_in_its_resources() {
        assert_eq!(
            in_app(Path::new("/Applications/Dupin.app/Contents/MacOS/dupin")),
            Some(PathBuf::from(
                "/Applications/Dupin.app/Contents/Resources/ThirdPartyNotices.txt"
            ))
        );
        assert_eq!(in_app(Path::new("/repo/target/debug/dupin")), None);
    }
}
