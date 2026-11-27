use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
}

/// Creates `path` with `bytes` only if it doesn't exist, all at once: a hard link either
/// publishes the finished file or fails with `AlreadyExists`, so a crash or a racing writer
/// can't leave a partial file behind.
pub fn create_new_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let staged = Staged::write(dir, bytes)?;
    fs::hard_link(&staged.path, path)?;
    drop(staged);
    sync_dir(dir)
}

static SCRATCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Stops flushing writes to disk for the rest of the process, for a headless run or a fixture
/// whose stores are thrown away when it ends. Every write stays atomic; only a crash or power
/// loss could lose one.
pub fn scratch_process() {
    SCRATCH.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn durable() -> bool {
    !SCRATCH.load(std::sync::atomic::Ordering::Relaxed)
}

/// A temporary file in `tmp`, written and flushed, ready to be renamed into place.
pub struct Staged {
    path: PathBuf,
    committed: bool,
}

impl Staged {
    pub fn write(tmp: &Path, bytes: &[u8]) -> io::Result<Self> {
        let mut name = [0u8; 16];
        getrandom::fill(&mut name).map_err(io::Error::other)?;
        let path = tmp.join(format!("{}.tmp", crate::hex(&name)));
        let staged = Self {
            path,
            committed: false,
        };
        let mut file = private_options().create_new(true).open(&staged.path)?;
        file.write_all(bytes)?;
        if durable() {
            file.sync_all()?;
        }
        Ok(staged)
    }

    /// Replaces `dest` in one step, so a reader sees the old file or the new one, never part.
    pub fn commit(mut self, dest: &Path) -> io::Result<()> {
        fs::rename(&self.path, dest)?;
        self.committed = true;
        sync_parent(dest)
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Makes a rename or removal in the directory durable, not only the file's contents.
pub fn sync_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(parent) => sync_dir(parent),
        None => Ok(()),
    }
}

pub fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    if durable() {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
