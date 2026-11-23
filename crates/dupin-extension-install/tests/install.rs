//! Installs from fixture Git repositories and folders on disk; nothing here touches the network.

use std::path::{Path, PathBuf};
use std::process::Command;

use dupin_extension_install::{Change, Error, Git, Installer, Integrity, LOCK_FILE, Lock, State};

/// Keeps the developer's own Git configuration out of the fixtures and the installer.
const ISOLATION: [(&str, &str); 7] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_AUTHOR_NAME", "Fixture"),
    ("GIT_AUTHOR_EMAIL", "fixture@example.invalid"),
    ("GIT_COMMITTER_NAME", "Fixture"),
    ("GIT_COMMITTER_EMAIL", "fixture@example.invalid"),
    ("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z"),
];

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .envs(ISOLATION)
        .output()
        .expect("git is installed");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

struct Fixture {
    _root: tempfile::TempDir,
    repo: PathBuf,
    installer: Installer,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        let installer = Installer {
            extensions: root.path().join("config/extensions"),
            lock: root.path().join("data").join(LOCK_FILE),
            git: Git {
                env: ISOLATION
                    .iter()
                    .map(|(key, value)| ((*key).into(), (*value).into()))
                    .collect(),
                ..Git::default()
            },
        };
        Self {
            _root: root,
            repo,
            installer,
        }
    }

    fn url(&self) -> String {
        format!("file://{}", self.repo.display())
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.repo.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn package(&self, name: &str, keyword: &str, capabilities: &str) {
        self.write(
            "package.json",
            &format!(
                r#"{{"name":"{name}","version":"1.0.0","scripts":{{"postinstall":"touch pwned"}},"dupin":{{"title":"Word Count","keyword":"{keyword}","capabilities":{capabilities}}}}}"#
            ),
        );
        self.write("main.js", "export function search() { return []; }\n");
    }

    fn commit(&self, message: &str) -> String {
        git(&self.repo, &["add", "--all"]);
        git(&self.repo, &["commit", "--quiet", "--allow-empty", "-m", message]);
        git(&self.repo, &["rev-parse", "HEAD"])
    }

    fn tag(&self, tag: &str) {
        git(&self.repo, &["tag", "-a", "-m", tag, tag]);
    }

    fn install(&self, spec: &str) -> Result<Change, Error> {
        self.installer.install(spec, Path::new("/"), &|_| None)
    }

    fn lock(&self) -> Lock {
        Lock::read(&self.installer.lock).unwrap()
    }

    fn installed(&self, name: &str) -> PathBuf {
        self.installer.extensions.join(name)
    }

    fn leftovers(&self) -> Vec<String> {
        std::fs::read_dir(&self.installer.extensions)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[test]
fn installs_the_newest_version_tag_and_pins_its_commit() {
    let fixture = Fixture::new();
    fixture.package("word-count", "wc", "{}");
    fixture.write(".gitattributes", "*.js filter=never-run\n");
    let first = fixture.commit("one");
    fixture.tag("v1.0.0");
    fixture.write("lib/rows.js", "export const rows = [];\n");
    let second = fixture.commit("two");
    fixture.tag("v1.10.0");
    fixture.tag("nightly");
    fixture.write(
        "main.js",
        "export function search() { return [{ title: 'unreleased' }]; }\n",
    );
    fixture.commit("three");

    let change = fixture.install(&fixture.url()).unwrap();
    assert_eq!(change.name, "word-count");
    assert_eq!(change.entry.commit.as_deref(), Some(second.as_str()));
    assert_eq!(change.entry.rev.as_deref(), Some("v1.10.0"));
    assert_eq!(change.entry.source, fixture.url());
    let folder = fixture.installed("word-count");
    assert!(folder.join("lib/rows.js").is_file());
    assert!(!folder.join(".git").exists());
    assert!(!folder.join("pwned").exists());
    assert_eq!(
        std::fs::read_to_string(folder.join("main.js")).unwrap(),
        "export function search() { return []; }\n"
    );
    assert_eq!(
        dupin_extension_install::content_hash(&folder).unwrap(),
        change.entry.content_hash
    );
    assert_eq!(fixture.lock().packages["word-count"], change.entry);
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());

    let pinned = fixture.install(&format!("{}@{first}", fixture.url())).unwrap();
    assert_eq!(pinned.entry.commit.as_deref(), Some(first.as_str()));
    assert!(!fixture.installed("word-count").join("lib").exists());

    let branch = fixture.install(&format!("{}@main", fixture.url())).unwrap();
    assert_eq!(branch.entry.rev.as_deref(), Some("main"));
    assert!(
        std::fs::read_to_string(fixture.installed("word-count").join("main.js"))
            .unwrap()
            .contains("unreleased")
    );
}

#[test]
fn without_version_tags_the_default_branch_is_installed() {
    let fixture = Fixture::new();
    fixture.package("word-count", "wc", "{}");
    let head = fixture.commit("one");
    fixture.tag("latest");
    let change = fixture.install(&fixture.url()).unwrap();
    assert_eq!(change.entry.commit.as_deref(), Some(head.as_str()));
    assert_eq!(change.entry.rev, None);
}

#[test]
fn updates_report_scope_changes_and_hold_added_scopes_for_consent() {
    let fixture = Fixture::new();
    fixture.package(
        "weather",
        "wx",
        r#"{"http":["https://api.example.com"],"clipboard":true}"#,
    );
    fixture.commit("one");
    fixture.tag("v1.0.0");
    fixture.install(&fixture.url()).unwrap();

    let mut lock = fixture.lock();
    let entry = lock.packages.get_mut("weather").unwrap();
    entry.granted = vec!["clipboard".into(), "http:https://api.example.com".into()];
    lock.write(&fixture.installer.lock).unwrap();

    let up_to_date = fixture.installer.update(Some("weather"), &|_| None).unwrap();
    assert!(up_to_date[0].1.as_ref().unwrap().unchanged());

    fixture.package(
        "weather",
        "wx",
        r#"{"http":["https://api.example.com"],"files":["~/Notes"]}"#,
    );
    let second = fixture.commit("two");
    fixture.tag("v2.0.0");
    let updated = fixture.installer.update(None, &|_| None).unwrap();
    let change = updated[0].1.as_ref().unwrap();
    assert_eq!(change.entry.commit.as_deref(), Some(second.as_str()));
    assert_eq!(change.added, ["files:~/Notes"]);
    assert_eq!(change.removed, ["clipboard"]);
    let entry = &fixture.lock().packages["weather"];
    assert_eq!(entry.granted, ["http:https://api.example.com"]);
    assert_eq!(entry.pending_consent, ["files:~/Notes"]);
    assert_eq!(entry.requested, ["files:~/Notes", "http:https://api.example.com"]);

    assert_eq!(
        fixture
            .installer
            .update(Some("absent"), &|_| None)
            .unwrap_err()
            .code,
        "not-installed"
    );
}

#[test]
fn unsafe_or_unbuilt_packages_are_refused_before_anything_is_placed() {
    let fixture = Fixture::new();
    fixture.package("word-count", "wc", "{}");
    std::os::unix::fs::symlink("/etc/hosts", fixture.repo.join("hosts")).unwrap();
    fixture.commit("link");
    let error = fixture.install(&fixture.url()).unwrap_err();
    assert_eq!(
        (error.code, error.file.as_deref()),
        ("invalid-package-path", Some("hosts"))
    );

    std::fs::remove_file(fixture.repo.join("hosts")).unwrap();
    let blob = fixture.commit("unlink");
    git(
        &fixture.repo,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{blob},vendor"),
        ],
    );
    git(&fixture.repo, &["commit", "--quiet", "-m", "submodule"]);
    assert_eq!(
        fixture.install(&fixture.url()).unwrap_err().file.as_deref(),
        Some("vendor")
    );
    git(&fixture.repo, &["rm", "--cached", "--quiet", "vendor"]);

    fixture.write(
        "package.json",
        r#"{"name":"word-count","version":"1.0.0","dupin":{"title":"Word Count","keyword":"wc","main":"src/main.ts"}}"#,
    );
    fixture.commit("typescript");
    let error = fixture.install(&fixture.url()).unwrap_err();
    assert_eq!(error.field.as_deref(), Some("dupin.main"));
    assert!(error.hint.unwrap().contains("built JavaScript"));

    fixture.write(
        "package.json",
        r#"{"name":"word-count","version":"1.0.0","dupin":{"title":"Word Count","keyword":"wc","main":"dist/main.js"}}"#,
    );
    fixture.commit("unbuilt");
    assert_eq!(fixture.install(&fixture.url()).unwrap_err().code, "missing-entry");

    fixture.write(
        "package.json",
        r#"{"name":"../escape","version":"1","dupin":{"title":"x","keyword":"x"}}"#,
    );
    fixture.commit("escape");
    assert_eq!(
        fixture.install(&fixture.url()).unwrap_err().field.as_deref(),
        Some("name")
    );

    assert_eq!(
        fixture
            .install(&format!("{}@no-such-tag", fixture.url()))
            .unwrap_err()
            .code,
        "unknown-ref"
    );
    assert!(!fixture.installed("word-count").exists());
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
    assert!(fixture.lock().packages.is_empty());
}

#[test]
fn keywords_and_folders_belong_to_one_package() {
    let fixture = Fixture::new();
    fixture.package("word-count", "wc", "{}");
    fixture.commit("one");

    let error = fixture
        .installer
        .install(&fixture.url(), Path::new("/"), &|keyword| {
            (keyword == "wc").then(|| "Word Count (built in)".to_owned())
        })
        .unwrap_err();
    assert_eq!(error.code, "keyword-taken");

    let sibling = fixture.installed("counter");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("extension.toml"), "keyword = \"wc\"\n").unwrap();
    let error = fixture.install(&fixture.url()).unwrap_err();
    assert!(error.message.contains("counter"), "{error}");
    std::fs::remove_dir_all(&sibling).unwrap();

    std::fs::create_dir_all(fixture.installed("word-count")).unwrap();
    assert_eq!(fixture.install(&fixture.url()).unwrap_err().code, "folder-exists");
}

#[test]
fn tampering_disables_the_package_until_sync_restores_it() {
    let fixture = Fixture::new();
    fixture.package("word-count", "wc", "{}");
    fixture.commit("one");
    fixture.tag("v1.0.0");
    let change = fixture.install(&fixture.url()).unwrap();
    let folder = fixture.installed("word-count");
    let mut integrity = Integrity::new(fixture.installer.lock.clone());
    assert_eq!(integrity.check(&folder), None);
    assert_eq!(
        integrity.check(&fixture.installer.extensions.join("hand-made")),
        None
    );

    std::fs::write(folder.join("main.js"), "export function search() { steal(); }\n").unwrap();
    let problem = integrity.check(&folder).unwrap();
    assert!(problem.contains("extensions.lock"), "{problem}");
    assert!(matches!(
        fixture.installer.list().unwrap()[0].state,
        State::Modified(_)
    ));

    let synced = fixture.installer.sync(&|_| None).unwrap();
    assert_eq!(synced.len(), 1);
    assert_eq!(
        synced[0].1.as_ref().unwrap().entry.content_hash,
        change.entry.content_hash
    );
    assert_eq!(integrity.check(&folder), None);

    std::fs::remove_dir_all(&folder).unwrap();
    assert_eq!(fixture.installer.list().unwrap()[0].state, State::Missing);
    fixture.installer.sync(&|_| None).unwrap();
    assert_eq!(fixture.installer.list().unwrap()[0].state, State::Installed);

    std::fs::write(&fixture.installer.lock, "not = [valid").unwrap();
    assert!(integrity.check(&folder).unwrap().contains("disabled"));
}

#[test]
fn folders_install_as_copies_and_remove_cleanly() {
    let fixture = Fixture::new();
    let source = fixture.repo.join("My Extension");
    std::fs::create_dir_all(source.join("node_modules/esbuild")).unwrap();
    std::fs::write(source.join("node_modules/esbuild/huge.bin"), "x").unwrap();
    std::fs::write(
        source.join("package.json"),
        r#"{"name":"notes","version":"0.1.0","dupin":{"title":"Notes","keyword":"nt"}}"#,
    )
    .unwrap();
    std::fs::write(
        source.join("main.js"),
        "export function search() { return []; }\n",
    )
    .unwrap();

    let change = fixture
        .installer
        .install("My Extension", &fixture.repo, &|_| None)
        .unwrap();
    assert_eq!(change.entry.commit, None);
    assert_eq!(change.entry.source, source.display().to_string());
    assert!(fixture.installed("notes").join("main.js").is_file());
    assert!(!fixture.installed("notes").join("node_modules").exists());

    std::fs::write(
        source.join("main.js"),
        "export function search() { return [{ title: 'new' }]; }\n",
    )
    .unwrap();
    let updated = fixture.installer.update(Some("notes"), &|_| None).unwrap();
    assert!(!updated[0].1.as_ref().unwrap().unchanged());

    std::os::unix::fs::symlink("/etc/hosts", source.join("hosts")).unwrap();
    assert_eq!(
        fixture.installer.update(Some("notes"), &|_| None).unwrap()[0]
            .1
            .as_ref()
            .unwrap_err()
            .code,
        "invalid-package-path"
    );

    let removed = fixture.installer.remove("notes").unwrap();
    assert_eq!(removed.keyword, "nt");
    assert!(!fixture.installed("notes").exists());
    assert!(fixture.lock().packages.is_empty());
    assert!(fixture.leftovers().is_empty(), "{:?}", fixture.leftovers());
    assert_eq!(
        fixture.installer.remove("notes").unwrap_err().code,
        "not-installed"
    );
}
