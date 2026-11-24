//! Moving a staged package into the extensions directory so the launcher's watcher never sees a
//! half-written folder, or, on macOS, a moment without the package.

use std::io;
use std::path::Path;

/// Puts `staged` at `target`. When `target` exists, the two swap, so `staged` then holds the
/// previous package for the caller to delete.
pub(crate) fn swap_into(staged: &Path, target: &Path) -> io::Result<()> {
    if target.symlink_metadata().is_err() {
        return std::fs::rename(staged, target);
    }
    exchange(staged, target)
}

#[cfg(target_os = "macos")]
fn exchange(a: &Path, b: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let a = CString::new(a.as_os_str().as_bytes())?;
    let b = CString::new(b.as_os_str().as_bytes())?;
    // SAFETY: both pointers are NUL-terminated paths that outlive the call.
    if unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn exchange(a: &Path, b: &Path) -> io::Result<()> {
    let aside = a.with_extension("previous");
    std::fs::rename(b, &aside)?;
    if let Err(error) = std::fs::rename(a, b) {
        let _ = std::fs::rename(&aside, b);
        return Err(error);
    }
    std::fs::rename(&aside, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replacement_swaps_with_the_old_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (staged, target) = (dir.path().join(".staged"), dir.path().join("pkg"));
        std::fs::create_dir(&staged).unwrap();
        std::fs::write(staged.join("v"), "1").unwrap();
        swap_into(&staged, &target).unwrap();
        assert!(!staged.exists());

        std::fs::create_dir(&staged).unwrap();
        std::fs::write(staged.join("v"), "2").unwrap();
        swap_into(&staged, &target).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("v")).unwrap(), "2");
        assert_eq!(std::fs::read_to_string(staged.join("v")).unwrap(), "1");
    }
}
