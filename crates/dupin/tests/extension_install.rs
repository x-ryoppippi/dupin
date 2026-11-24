//! `dupin ext install` as the `dupin` executable runs it, against directories of its own.

use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use dupin_extension_bridge::{Origin, PackageId, storage};

struct Dirs(tempfile::TempDir);

impl Dirs {
    fn new() -> Self {
        let dirs = Self(tempfile::tempdir().unwrap());
        for sub in ["home", "config", "data", "run"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(dirs.path(sub))
                .unwrap();
        }
        dirs
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.0.path().join(sub)
    }

    fn storage(&self) -> PathBuf {
        self.path("data").join("dupin").join("storage")
    }

    /// A package named `name` in a folder of its own, ready to install.
    fn package(&self, name: &str) -> PathBuf {
        let folder = self.0.path().join("packages").join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("package.json"),
            serde_json::json!({
                "name": name,
                "version": "0.1.0",
                "dupin": { "title": "Notes", "keyword": name },
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(folder.join("main.js"), "export const search = () => [];\n").unwrap();
        folder
    }

    fn dupin(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_dupin"))
            .args(args)
            .current_dir(self.0.path())
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_RUNTIME_DIR", self.path("run"))
            .output()
            .unwrap()
    }
}

fn namespace(name: &str) -> String {
    storage::namespace(&PackageId::new(name).unwrap(), Origin::User)
}

fn installed(dirs: &Dirs, name: &str) -> bool {
    dirs.path("config").join("dupin/extensions").join(name).exists()
}

fn leave_data(storage: &Path, name: &str) -> PathBuf {
    let left = storage.join(namespace(name));
    std::fs::create_dir_all(left.join("records")).unwrap();
    std::fs::write(left.join("records").join("left-behind"), b"sealed").unwrap();
    left
}

#[test]
fn installing_a_name_afresh_deletes_what_an_earlier_package_of_that_name_stored() {
    let dirs = Dirs::new();
    let left = leave_data(&dirs.storage(), "notes");
    let package = dirs.package("notes");

    let output = dirs.dupin(&["ext", "install", package.to_str().unwrap()]);
    assert!(output.status.success(), "{output:?}");
    assert!(installed(&dirs, "notes"));
    assert!(!left.exists(), "the new package can't read the old one's data");
}

#[test]
fn installing_a_name_afresh_is_undone_when_its_old_data_cant_be_deleted() {
    let dirs = Dirs::new();
    let held = dupin_storage::Storage::new(dirs.storage(), dupin_storage::MemoryKey::random())
        .open(&namespace("notes"), dupin_storage::Limits::default())
        .unwrap();
    let package = dirs.package("notes");

    let output = dirs.dupin(&["ext", "install", package.to_str().unwrap(), "--json"]);
    assert!(!output.status.success(), "{output:?}");
    let report = String::from_utf8_lossy(&output.stdout) + String::from_utf8_lossy(&output.stderr);
    assert!(report.contains("data-undeleted"), "{report}");
    assert!(!installed(&dirs, "notes"));
    let listed = dirs.dupin(&["ext", "list", "--json"]);
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(listed["packages"], serde_json::json!([]));
    assert!(dirs.storage().join(namespace("notes")).exists());
    drop(held);
}
