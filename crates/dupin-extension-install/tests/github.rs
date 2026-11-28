//! Installs `github:` sources from a loopback server that answers as GitHub's REST API and
//! archive host do, with tarballs `git archive` makes from fixture repositories or that the tests
//! write by hand; nothing here touches the network or needs Git to install.

use dupin_spawn::Command;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dupin_extension_install::{Error, Git, GitHub, Installer, Lock, LockStore, content_hash, guard_path};

/// The list in a JSON file, so each installer a test makes for one name shares it.
struct FileLock(PathBuf);

impl LockStore for FileLock {
    fn read(&self) -> Result<Lock, Error> {
        match std::fs::read(&self.0) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|error| Error::new("invalid-lock", error.to_string()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Lock::new()),
            Err(error) => Err(Error::new("io-error", error.to_string())),
        }
    }

    fn write(&self, lock: &Lock) -> Result<(), Error> {
        lock.validate()?;
        std::fs::create_dir_all(self.0.parent().unwrap()).unwrap();
        std::fs::write(&self.0, serde_json::to_vec(lock).unwrap())
            .map_err(|error| Error::new("io-error", error.to_string()))
    }
}
use serde_json::{Value, json};

const OWNER: &str = "someone";
const REPO: &str = "word-count";
const SPEC: &str = "github:someone/word-count";

const ISOLATION: [(&str, &str); 7] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_AUTHOR_NAME", "Fixture"),
    ("GIT_AUTHOR_EMAIL", "fixture@example.invalid"),
    ("GIT_COMMITTER_NAME", "Fixture"),
    ("GIT_COMMITTER_EMAIL", "fixture@example.invalid"),
    ("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z"),
];

fn git_bytes(dir: &Path, args: &[&str]) -> Vec<u8> {
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
    output.stdout
}

fn git(dir: &Path, args: &[&str]) -> String {
    String::from_utf8(git_bytes(dir, args)).unwrap().trim().to_owned()
}

struct Response {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
}

fn json_response(value: &Value) -> Response {
    Response {
        status: 200,
        location: None,
        body: serde_json::to_vec(value).unwrap(),
    }
}

#[derive(Clone, Debug)]
struct Seen {
    host: String,
    path: String,
    authorization: Option<String>,
}

type Routes = Arc<Mutex<HashMap<String, Response>>>;

/// A loopback HTTP server answering from `routes` by path and query, 404 otherwise.
struct Server {
    port: u16,
    routes: Routes,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Routes = Arc::default();
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let (shared_routes, log) = (Arc::clone(&routes), Arc::clone(&seen));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let routes = Arc::clone(&shared_routes);
                let log = Arc::clone(&log);
                std::thread::spawn(move || serve(stream, &routes, &log));
            }
        });
        Self { port, routes, seen }
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn set(&self, path: impl Into<String>, response: Response) {
        self.routes.lock().unwrap().insert(path.into(), response);
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn serve(stream: TcpStream, routes: &Routes, log: &Mutex<Vec<Seen>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or_default().to_owned();
    let (mut host, mut authorization) = (String::new(), None);
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.to_ascii_lowercase().as_str() {
                "host" => host = value.trim().to_owned(),
                "authorization" => authorization = Some(value.trim().to_owned()),
                _ => {}
            }
        }
    }
    log.lock().unwrap().push(Seen {
        host,
        path: path.clone(),
        authorization,
    });
    let mut stream = stream;
    let routes = routes.lock().unwrap();
    let not_found = Response {
        status: 404,
        location: None,
        body: br#"{"message":"Not Found"}"#.to_vec(),
    };
    let response = routes.get(&path).unwrap_or(&not_found);
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        response.body.len()
    );
    if let Some(location) = &response.location {
        head.push_str(&format!("Location: {location}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&response.body);
}

/// A fixture repository, published to the server as GitHub would show it.
struct Fixture {
    root: tempfile::TempDir,
    repo: PathBuf,
    server: Server,
    /// Where the API sends tarball downloads: another host name for the same server, so the
    /// token's scope can be checked.
    archive_origin: String,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        let server = Server::start();
        let archive_origin = format!("http://localhost:{}", server.port);
        Self {
            root,
            repo,
            server,
            archive_origin,
        }
    }

    fn installer(&self, name: &str, token: Option<&str>) -> Installer {
        Installer {
            extensions: self.root.path().join(name).join("extensions"),
            lock: Box::new(FileLock(self.root.path().join(name).join("installed.json"))),
            guard: guard_path(&self.root.path().join(name)),
            git: Git {
                env: ISOLATION
                    .iter()
                    .map(|(key, value)| ((*key).into(), (*value).into()))
                    .collect(),
                ..Git::default()
            },
            github: GitHub {
                api: self.server.origin(),
                curl: PathBuf::from("/usr/bin/curl"),
                timeout: Duration::from_secs(30),
                token: token.map(str::to_owned),
                ..GitHub::default()
            },
            bundled: Default::default(),
        }
    }

    /// An installer that can't run Git at all.
    fn without_git(&self, name: &str) -> Installer {
        let mut installer = self.installer(name, None);
        installer.git.program = self.root.path().join("no-git-here");
        installer
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.repo.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn package(&self, name: &str) {
        self.write(
            "package.json",
            &format!(
                r#"{{"name":"{name}","version":"1.0.0","scripts":{{"postinstall":"touch pwned"}},"dupin":{{"title":"Word Count","keyword":"wc"}}}}"#
            ),
        );
        self.write("main.js", "export function search() { return []; }\n");
    }

    fn commit(&self, message: &str) -> String {
        git(&self.repo, &["add", "--all"]);
        git(&self.repo, &["commit", "--quiet", "--allow-empty", "-m", message]);
        let head = git(&self.repo, &["rev-parse", "HEAD"]);
        self.publish();
        head
    }

    fn tag(&self, tag: &str) {
        git(&self.repo, &["tag", "-a", "-m", tag, tag]);
        self.publish();
    }

    fn repo_path(&self) -> String {
        format!("/repos/{OWNER}/{REPO}")
    }

    /// Answers for every ref and commit of the repository as it is now, with `padding` more
    /// lightweight tags on its first commit.
    fn publish_with_tags(&self, padding: usize) {
        self.publish();
        let first = git(&self.repo, &["rev-list", "--max-parents=0", "HEAD"]);
        let mut refs = self.tag_refs();
        refs.extend((0..padding).map(
            |n| json!({"ref": format!("refs/tags/v0.0.{n}"), "object": {"type": "commit", "sha": first}}),
        ));
        self.server.set(
            format!("{}/git/matching-refs/tags", self.repo_path()),
            json_response(&Value::Array(refs)),
        );
    }

    fn tag_refs(&self) -> Vec<Value> {
        let refs = git(
            &self.repo,
            &[
                "for-each-ref",
                "--format=%(refname) %(objecttype) %(objectname)",
                "refs/tags",
            ],
        );
        refs.lines()
            .map(|line| {
                let [name, kind, object] = line.split(' ').collect::<Vec<_>>()[..] else {
                    panic!("{line}");
                };
                json!({"ref": name, "object": {"type": kind, "sha": object}})
            })
            .collect()
    }

    /// Answers for every ref and commit of the repository as it is now.
    fn publish(&self) {
        let base = self.repo_path();
        self.server
            .set(base.clone(), json_response(&json!({"default_branch": "main"})));
        let refs = git(
            &self.repo,
            &["for-each-ref", "--format=%(refname) %(objecttype) %(objectname)"],
        );
        for line in refs.lines() {
            let [name, kind, object] = line.split(' ').collect::<Vec<_>>()[..] else {
                continue;
            };
            let short = name.strip_prefix("refs/").unwrap();
            self.server.set(
                format!("{base}/git/ref/{short}"),
                json_response(&json!({"ref": name, "object": {"type": kind, "sha": object}})),
            );
            if kind == "tag" {
                let commit = git(&self.repo, &["rev-parse", &format!("{name}^{{commit}}")]);
                self.server.set(
                    format!("{base}/git/tags/{object}"),
                    json_response(&json!({"sha": object, "object": {"type": "commit", "sha": commit}})),
                );
            }
        }
        self.server.set(
            format!("{base}/git/matching-refs/tags"),
            json_response(&Value::Array(self.tag_refs())),
        );
        let tagged = git(
            &self.repo,
            &[
                "for-each-ref",
                "--format=%(refname:strip=2) %(*objectname) %(objectname)",
                "refs/tags",
            ],
        );
        let tags: Vec<Value> = tagged
            .lines()
            .map(|line| {
                let [name, peeled, object] = line.split(' ').collect::<Vec<_>>()[..] else {
                    panic!("{line}");
                };
                let commit = if peeled.is_empty() { object } else { peeled };
                json!({"name": name, "commit": {"sha": commit}})
            })
            .collect();
        for (page, chunk) in tags.chunks(100).chain([&[][..]]).enumerate() {
            self.server.set(
                format!("{base}/tags?per_page=100&page={}", page + 1),
                json_response(&Value::Array(chunk.to_vec())),
            );
        }
        let names = git(
            &self.repo,
            &[
                "for-each-ref",
                "--format=%(refname:strip=2)",
                "refs/heads",
                "refs/tags",
            ],
        );
        let commits = git(&self.repo, &["rev-list", "--all"]);
        for commit in commits.lines() {
            self.publish_commit(commit);
            for name in names.lines() {
                let tip = git(&self.repo, &["rev-parse", &format!("{name}^{{commit}}")]);
                let reached = Command::new("git")
                    .args(["merge-base", "--is-ancestor", commit, &tip])
                    .current_dir(&self.repo)
                    .envs(ISOLATION)
                    .status()
                    .unwrap()
                    .success();
                let status = match (commit == tip, reached) {
                    (true, _) => "identical",
                    (false, true) => "behind",
                    (false, false) => "diverged",
                };
                self.compare(name, commit, status);
            }
        }
    }

    /// Answers GitHub's comparison of the branch or tag `base` with `commit`.
    fn compare(&self, base: &str, commit: &str, status: &str) {
        self.server.set(
            format!("{}/compare/{base}...{commit}", self.repo_path()),
            json_response(&json!({ "status": status })),
        );
    }

    fn publish_commit(&self, commit: &str) {
        let base = self.repo_path();
        self.server.set(
            format!("{base}/git/commits/{commit}"),
            json_response(&json!({"sha": commit})),
        );
        let listing = git(&self.repo, &["ls-tree", "-r", "-t", "-l", "--full-tree", commit]);
        let tree: Vec<Value> = listing
            .lines()
            .map(|line| {
                let (head, path) = line.split_once('\t').unwrap();
                let [mode, kind, sha, size] = head.split_whitespace().collect::<Vec<_>>()[..] else {
                    panic!("{line}");
                };
                let mut entry = json!({"path": path, "mode": mode, "type": kind, "sha": sha});
                if kind == "blob" {
                    entry["size"] = json!(size.parse::<u64>().unwrap());
                }
                entry
            })
            .collect();
        self.server.set(
            format!("{base}/git/trees/{commit}?recursive=1"),
            json_response(&json!({"sha": commit, "tree": tree, "truncated": false})),
        );
        let prefix = format!("{OWNER}-{REPO}-{}/", &commit[..7]);
        let archive = git_bytes(
            &self.repo,
            &[
                "archive",
                "--format=tar.gz",
                &format!("--prefix={prefix}"),
                commit,
            ],
        );
        self.serve_archive(commit, archive);
    }

    /// Serves `archive` as GitHub's tarball of `commit`, through the API's redirect.
    fn serve_archive(&self, commit: &str, archive: Vec<u8>) {
        let codeload = format!("/codeload/{OWNER}/{REPO}/legacy.tar.gz/{commit}");
        self.server.set(
            format!("{}/tarball/{commit}", self.repo_path()),
            Response {
                status: 302,
                location: Some(format!("{}{codeload}", self.archive_origin)),
                body: Vec::new(),
            },
        );
        self.server.set(
            codeload,
            Response {
                status: 200,
                location: None,
                body: archive,
            },
        );
    }

    /// Replaces what GitHub lists for `commit`'s tree.
    fn serve_tree(&self, commit: &str, tree: Value) {
        self.server.set(
            format!("{}/git/trees/{commit}?recursive=1", self.repo_path()),
            json_response(&json!({"sha": commit, "tree": tree, "truncated": false})),
        );
    }

    fn downloads(&self) -> usize {
        self.server
            .seen()
            .iter()
            .filter(|seen| seen.path.starts_with("/codeload/"))
            .count()
    }
}

fn install(installer: &Installer, spec: &str) -> Result<dupin_extension_install::Change, Error> {
    installer.install(spec, Path::new("/"), &|_| None)
}

fn leftovers(installer: &Installer) -> Vec<String> {
    std::fs::read_dir(&installer.extensions)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_pinned_commit_must_be_one_the_repository_publishes() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    let first = fixture.commit("one");
    git(&fixture.repo, &["switch", "--quiet", "-c", "released"]);
    fixture.write("main.js", "export function search() { return []; }\n");
    let tagged = fixture.commit("tagged off the default branch");
    fixture.tag("v2.0.0");
    git(&fixture.repo, &["switch", "--quiet", "-c", "fork", "main"]);
    fixture.write("main.js", "export function search() { steal(); }\n");
    let forked = fixture.commit("pushed to a fork");
    git(&fixture.repo, &["switch", "--quiet", "main"]);
    git(&fixture.repo, &["branch", "--quiet", "-D", "fork", "released"]);
    fixture.publish();
    // GitHub serves a fork's commits through the repository it was forked from.
    fixture.publish_commit(&forked);
    fixture.compare("main", &forked, "ahead");

    let github = fixture.without_git("github");
    let asked = fixture.server.seen().len();
    let error = install(&github, &format!("{SPEC}@{forked}")).unwrap_err();
    assert_eq!(error.code, "unknown-ref", "{error:?}");
    assert!(error.message.contains("default branch or a tag"), "{error:?}");
    assert_eq!(fixture.downloads(), 0);
    let tag_requests: Vec<String> = fixture.server.seen()[asked..]
        .iter()
        .map(|seen| seen.path.clone())
        .filter(|path| path.contains("/tags"))
        .collect();
    assert_eq!(
        tag_requests,
        [format!("{}/tags?per_page=100&page=1", fixture.repo_path())],
        "the tag list names each tag's commit, so no annotated tag is peeled one by one"
    );

    let pinned = install(&github, &format!("{SPEC}@{first}")).unwrap();
    assert_eq!(pinned.entry.commit.as_deref(), Some(first.as_str()));
    let released = fixture.without_git("released");
    let pinned = install(&released, &format!("{SPEC}@{tagged}")).unwrap();
    assert_eq!(pinned.entry.commit.as_deref(), Some(tagged.as_str()));

    let pull = install(&fixture.without_git("pull"), &format!("{SPEC}@refs/pull/1/head")).unwrap_err();
    assert_eq!(pull.code, "invalid-source", "{pull:?}");
}

#[test]
fn a_package_installed_from_a_branch_syncs_from_it_after_the_branch_moves_on() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    fixture.commit("one");
    git(&fixture.repo, &["switch", "--quiet", "-c", "dev"]);
    fixture.write("lib/rows.js", "export const rows = [];\n");
    let dev = fixture.commit("on dev only");

    let github = fixture.without_git("github");
    let installed = install(&github, &format!("{SPEC}@dev")).unwrap();
    assert_eq!(installed.entry.rev.as_deref(), Some("dev"));
    assert_eq!(installed.entry.commit.as_deref(), Some(dev.as_str()));

    fixture.write("lib/rows.js", "export const rows = [1];\n");
    fixture.commit("dev moves on");
    let folder = github.extensions.join("word-count");
    std::fs::write(folder.join("main.js"), "tampered").unwrap();
    let synced = github.sync(&|_| None).unwrap();
    let restored = synced[0].1.as_ref().unwrap();
    assert_eq!(restored.entry.commit.as_deref(), Some(dev.as_str()));
    assert_eq!(restored.entry.rev.as_deref(), Some("dev"));
    assert_eq!(content_hash(&folder).unwrap(), installed.entry.content_hash);

    std::fs::remove_dir_all(&folder).unwrap();
    let synced = github.sync(&|_| None).unwrap();
    assert!(synced[0].1.is_ok(), "{synced:?}");
    assert_eq!(content_hash(&folder).unwrap(), installed.entry.content_hash);
}

#[test]
fn a_github_install_needs_no_git_and_locks_as_the_git_path_does() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    fixture.write("bin/tool.sh", "#!/bin/sh\n");
    fixture.write(".gitattributes", "*.js filter=never-run\n");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        fixture.repo.join("bin/tool.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fixture.commit("one");
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

    let github = fixture.without_git("github");
    let change = install(&github, SPEC).unwrap();
    assert_eq!(change.name, "word-count");
    assert_eq!(change.entry.source, SPEC);
    assert_eq!(change.entry.rev.as_deref(), Some("v1.10.0"));
    assert_eq!(change.entry.commit.as_deref(), Some(second.as_str()));
    let folder = github.extensions.join("word-count");
    assert!(folder.join("lib/rows.js").is_file());
    assert!(!folder.join("pwned").exists());
    let mode = folder
        .join("bin/tool.sh")
        .metadata()
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    assert_eq!(content_hash(&folder).unwrap(), change.entry.content_hash);
    assert!(leftovers(&github).is_empty(), "{:?}", leftovers(&github));

    let git = fixture.installer("git", None);
    let url = format!("file://{}", fixture.repo.display());
    let cloned = install(&git, &url).unwrap();
    assert_eq!(cloned.entry.commit, change.entry.commit);
    assert_eq!(cloned.entry.rev, change.entry.rev);
    assert_eq!(cloned.entry.content_hash, change.entry.content_hash);

    std::fs::write(folder.join("main.js"), "tampered").unwrap();
    let restored = github.sync(&|_| None).unwrap();
    let (name, result) = &restored[0];
    assert_eq!(name, "word-count");
    let restored = result.as_ref().unwrap();
    assert_eq!(restored.entry.content_hash, change.entry.content_hash);
    assert_eq!(restored.entry.rev.as_deref(), Some("v1.10.0"));
    assert_eq!(content_hash(&folder).unwrap(), change.entry.content_hash);

    fixture.write(
        "main.js",
        "export function search() { return [{ title: 'two' }]; }\n",
    );
    let third = fixture.commit("four");
    fixture.tag("v2.0.0");
    let updated = github.update(None, &|_| None).unwrap();
    let updated = updated[0].1.as_ref().unwrap();
    assert_eq!(updated.entry.rev.as_deref(), Some("v2.0.0"));
    assert_eq!(updated.entry.commit.as_deref(), Some(third.as_str()));
    assert_eq!(github.lock.read().unwrap().packages["word-count"], updated.entry);
}

#[test]
fn refs_resolve_as_git_resolves_them() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    let first = fixture.commit("one");
    fixture.tag("latest");
    fixture.write("lib/rows.js", "export const rows = [];\n");
    let head = fixture.commit("two");
    let installer = fixture.without_git("github");

    let default = install(&installer, SPEC).unwrap();
    assert_eq!(default.entry.commit.as_deref(), Some(head.as_str()));
    assert_eq!(default.entry.rev, None);

    let tagged = install(&installer, &format!("{SPEC}@latest")).unwrap();
    assert_eq!(tagged.entry.commit.as_deref(), Some(first.as_str()));
    assert_eq!(tagged.entry.rev.as_deref(), Some("latest"));

    let branch = install(&installer, &format!("{SPEC}@main")).unwrap();
    assert_eq!(branch.entry.commit.as_deref(), Some(head.as_str()));
    assert_eq!(branch.entry.rev.as_deref(), Some("main"));

    let pinned = install(&installer, &format!("{SPEC}@{first}")).unwrap();
    assert_eq!(pinned.entry.commit.as_deref(), Some(first.as_str()));
    assert_eq!(pinned.entry.rev, None);
    assert!(!installer.extensions.join("word-count/lib").exists());

    let unknown = install(&installer, &format!("{SPEC}@nope")).unwrap_err();
    assert_eq!(unknown.code, "unknown-ref", "{unknown}");
    let short = install(&installer, &format!("{SPEC}@{}", &first[..7])).unwrap_err();
    assert!(short.hint.unwrap().contains("40-character"));
    let missing = install(&installer, &format!("{SPEC}@{}", "0".repeat(40))).unwrap_err();
    assert_eq!(missing.code, "unknown-ref");
    let no_repo = install(&installer, "github:someone/elsewhere").unwrap_err();
    assert_eq!(no_repo.code, "unknown-source", "{no_repo}");
    assert!(leftovers(&installer).is_empty());
}

#[test]
fn the_token_goes_to_the_api_host_only() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    fixture.commit("one");
    let installer = fixture.installer("github", Some("test-token-value"));
    install(&installer, SPEC).unwrap();
    let seen = fixture.server.seen();
    assert!(!seen.is_empty());
    for request in &seen {
        if request.path.starts_with("/codeload/") {
            assert!(request.host.starts_with("localhost"), "{request:?}");
            assert_eq!(request.authorization, None, "{request:?}");
        } else {
            assert_eq!(
                request.authorization.as_deref(),
                Some("Bearer test-token-value"),
                "{request:?}"
            );
        }
    }
    assert_eq!(fixture.downloads(), 1);

    let anonymous = fixture.installer("anonymous", None);
    install(&anonymous, SPEC).unwrap();
    assert!(
        fixture.server.seen()[seen.len()..]
            .iter()
            .all(|request| request.authorization.is_none())
    );
}

#[test]
fn files_the_archive_drops_or_rewrites_are_refused() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    fixture.write("notes.txt", "kept out of archives\n");
    fixture.write(".gitattributes", "notes.txt export-ignore\n");
    fixture.commit("one");
    let installer = fixture.without_git("github");
    let error = install(&installer, SPEC).unwrap_err();
    assert_eq!(error.code, "invalid-archive", "{error}");
    assert_eq!(error.file.as_deref(), Some("notes.txt"));

    fixture.write(".gitattributes", "main.js export-subst\n");
    fixture.write(
        "main.js",
        "// $Format:%H$\nexport function search() { return []; }\n",
    );
    fixture.commit("two");
    let error = install(&installer, SPEC).unwrap_err();
    assert_eq!(error.code, "invalid-archive", "{error}");
    assert_eq!(error.file.as_deref(), Some("main.js"));
    assert!(!installer.extensions.join("word-count").exists());
    assert!(leftovers(&installer).is_empty());
}

/// One tar entry, written as `git archive` would but with whatever the test needs.
struct Entry<'a> {
    path: &'a str,
    kind: u8,
    mode: u32,
    link: &'a str,
    data: &'a [u8],
}

fn file<'a>(path: &'a str, data: &'static [u8]) -> Entry<'a> {
    Entry {
        path,
        kind: b'0',
        mode: 0o664,
        link: "",
        data,
    }
}

fn header(path: &str, kind: u8, mode: u32, size: usize, link: &str) -> [u8; 512] {
    let mut header = [0u8; 512];
    header[..path.len()].copy_from_slice(path.as_bytes());
    let octal = |header: &mut [u8; 512], at: usize, width: usize, value: u64| {
        let text = format!("{value:0w$o}\0", w = width - 1);
        header[at..at + width].copy_from_slice(text.as_bytes());
    };
    octal(&mut header, 100, 8, u64::from(mode));
    octal(&mut header, 108, 8, 0);
    octal(&mut header, 116, 8, 0);
    octal(&mut header, 124, 12, size as u64);
    octal(&mut header, 136, 12, 0);
    header[156] = kind;
    header[157..157 + link.len()].copy_from_slice(link.as_bytes());
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[148..156].copy_from_slice(b"        ");
    let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    header
}

fn tarball(entries: &[Entry<'_>]) -> Vec<u8> {
    let mut tar = Vec::new();
    tar.extend_from_slice(&header("pax_global_header", b'g', 0o666, 52, ""));
    let mut global = b"52 comment=0000000000000000000000000000000000000000\n".to_vec();
    global.resize(512, 0);
    tar.extend_from_slice(&global);
    tar.extend_from_slice(&header("top/", b'5', 0o775, 0, ""));
    for entry in entries {
        tar.extend_from_slice(&header(
            entry.path,
            entry.kind,
            entry.mode,
            entry.data.len(),
            entry.link,
        ));
        let mut data = entry.data.to_vec();
        data.resize(entry.data.len().div_ceil(512) * 512, 0);
        tar.extend_from_slice(&data);
    }
    tar.extend_from_slice(&[0; 1024]);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&tar).unwrap();
    encoder.finish().unwrap()
}

fn blob(path: &str, data: &[u8]) -> Value {
    let mut hasher = sha1_smol::Sha1::new();
    hasher.update(format!("blob {}\0", data.len()).as_bytes());
    hasher.update(data);
    json!({"path": path, "mode": "100644", "type": "blob", "sha": hasher.digest().to_string(), "size": data.len()})
}

const MANIFEST: &[u8] =
    br#"{"name":"word-count","version":"1.0.0","dupin":{"title":"Word Count","keyword":"wc"}}"#;
const MAIN: &[u8] = b"export function search() { return []; }\n";

#[test]
fn unsafe_archives_are_refused_before_anything_is_placed() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    let commit = fixture.commit("one");
    let installer = fixture.without_git("github");
    let outside = fixture.root.path().join("github/escaped");
    let tree = json!([blob("package.json", MANIFEST), blob("main.js", MAIN)]);
    fixture.serve_tree(&commit, tree.clone());

    let cases: Vec<(&str, Vec<Entry<'_>>, &str)> = vec![
        (
            "a symlink",
            vec![Entry {
                path: "top/main.js",
                kind: b'2',
                mode: 0o777,
                link: "/etc/hosts",
                data: b"",
            }],
            "invalid-package-path",
        ),
        (
            "a path out of the package",
            vec![file("top/../escaped", b"x")],
            "invalid-archive",
        ),
        (
            "an absolute path",
            vec![file("/top/main.js", MAIN)],
            "invalid-package-path",
        ),
        (
            "a second top folder",
            vec![file("other/main.js", MAIN)],
            "invalid-archive",
        ),
        (
            "a device",
            vec![Entry {
                path: "top/main.js",
                kind: b'3',
                mode: 0o644,
                link: "",
                data: b"",
            }],
            "invalid-package-path",
        ),
        (
            "a hard link",
            vec![Entry {
                path: "top/main.js",
                kind: b'1',
                mode: 0o644,
                link: "top/package.json",
                data: b"",
            }],
            "invalid-package-path",
        ),
        (
            "a file the commit doesn't have",
            vec![
                file("top/package.json", MANIFEST),
                file("top/main.js", MAIN),
                file("top/extra.js", b"x"),
            ],
            "invalid-archive",
        ),
        (
            "a file with other contents",
            vec![
                file("top/package.json", MANIFEST),
                file("top/main.js", b"export const x = 1;\n"),
            ],
            "invalid-archive",
        ),
        (
            "a file with another mode",
            vec![
                file("top/package.json", MANIFEST),
                Entry {
                    mode: 0o775,
                    ..file("top/main.js", MAIN)
                },
            ],
            "invalid-archive",
        ),
    ];
    for (what, entries, code) in cases {
        fixture.serve_archive(&commit, tarball(&entries));
        let error = install(&installer, SPEC).expect_err(what);
        assert_eq!(error.code, code, "{what}: {error}");
        assert!(!installer.extensions.join("word-count").exists(), "{what}");
        assert!(!outside.exists(), "{what}");
        assert!(leftovers(&installer).is_empty(), "{what}");
    }

    fixture.serve_archive(
        &commit,
        tarball(&[file("top/package.json", MANIFEST), file("top/main.js", MAIN)]),
    );
    install(&installer, SPEC).unwrap();
}

#[test]
fn limits_apply_before_and_while_downloading() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    let commit = fixture.commit("one");
    let mut installer = fixture.without_git("github");

    for (what, entry, code) in [
        (
            "a symlink",
            json!({"path": "link", "mode": "120000", "type": "blob", "sha": "0".repeat(40), "size": 9}),
            "invalid-package-path",
        ),
        (
            "a submodule",
            json!({"path": "vendor", "mode": "160000", "type": "commit", "sha": "0".repeat(40)}),
            "invalid-package-path",
        ),
        (
            "a huge file",
            json!({"path": "big.bin", "mode": "100644", "type": "blob", "sha": "0".repeat(40), "size": 64 * 1024 * 1024}),
            "package-too-large",
        ),
        ("a .git folder", blob(".git/config", b""), "invalid-package-path"),
    ] {
        fixture.serve_tree(&commit, json!([blob("package.json", MANIFEST), entry]));
        let error = install(&installer, SPEC).expect_err(what);
        assert_eq!(error.code, code, "{what}: {error}");
    }
    assert_eq!(
        fixture.downloads(),
        0,
        "the tree is checked before the archive is fetched"
    );

    let many: Vec<Value> = (0..=1024).map(|n| blob(&format!("f{n}.js"), b"")).collect();
    fixture.serve_tree(&commit, Value::Array(many));
    assert_eq!(install(&installer, SPEC).unwrap_err().code, "package-too-large");

    fixture.serve_tree(
        &commit,
        json!([blob("package.json", MANIFEST), blob("main.js", MAIN)]),
    );
    let mut padding = Vec::new();
    for _ in 0..450 {
        padding.push(Entry {
            path: "pax_global_header",
            kind: b'g',
            mode: 0o666,
            link: "",
            data: &[b'\n'; 60 * 1024],
        });
    }
    fixture.serve_archive(&commit, tarball(&padding));
    let error = install(&installer, SPEC).unwrap_err();
    assert_eq!(error.code, "package-too-large", "{error}");

    installer.github.max_download_bytes = 1024;
    fixture.serve_archive(&commit, vec![0x1f; 64 * 1024]);
    let error = install(&installer, SPEC).unwrap_err();
    assert_eq!(error.code, "package-too-large", "{error}");
    assert!(leftovers(&installer).is_empty());
}

#[test]
fn rate_limits_and_bad_tokens_say_what_to_do() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    fixture.commit("one");
    let installer = fixture.without_git("github");
    fixture.server.set(
        fixture.repo_path(),
        Response {
            status: 403,
            location: None,
            body: br#"{"message":"API rate limit exceeded"}"#.to_vec(),
        },
    );
    let error = install(&installer, SPEC).unwrap_err();
    assert_eq!(error.code, "download-failed");
    assert!(error.hint.unwrap().contains("GITHUB_TOKEN"));

    let unreachable = Installer {
        github: GitHub {
            api: "http://127.0.0.1:9".to_owned(),
            ..installer.github.clone()
        },
        ..fixture.without_git("unreachable")
    };
    assert_eq!(install(&unreachable, SPEC).unwrap_err().code, "download-failed");
}

#[test]
fn the_newest_version_is_found_among_more_tags_than_a_page_holds() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    let first = fixture.commit("one");
    fixture.tag("v1.0.0");
    fixture.publish_with_tags(999);
    let installer = fixture.without_git("github");
    let installed = install(&installer, SPEC).unwrap();
    assert_eq!(installed.entry.rev.as_deref(), Some("v1.0.0"));
    assert_eq!(installed.entry.commit.as_deref(), Some(first.as_str()));

    fixture.write("lib/rows.js", "export const rows = [];\n");
    let second = fixture.commit("two");
    fixture.tag("v1.1.0");
    fixture.publish_with_tags(1500);
    let updated = installer.update(None, &|_| None).unwrap();
    let updated = updated[0].1.as_ref().unwrap();
    assert_eq!(updated.entry.rev.as_deref(), Some("v1.1.0"));
    assert_eq!(updated.entry.commit.as_deref(), Some(second.as_str()));
}

#[test]
fn an_lfs_object_in_the_archive_is_explained() {
    let fixture = Fixture::new();
    fixture.package("word-count");
    let commit = fixture.commit("one");
    let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";
    fixture.serve_tree(
        &commit,
        json!([
            blob("package.json", MANIFEST),
            blob("main.js", MAIN),
            blob("icon.png", pointer)
        ]),
    );
    fixture.serve_archive(
        &commit,
        tarball(&[
            file("top/icon.png", b"\x89PNG the real image"),
            file("top/main.js", MAIN),
            file("top/package.json", MANIFEST),
        ]),
    );
    let installer = fixture.without_git("github");
    let error = install(&installer, SPEC).unwrap_err();
    assert_eq!(error.code, "invalid-archive", "{error}");
    assert_eq!(error.file.as_deref(), Some("icon.png"));
    assert!(error.hint.unwrap().contains("LFS"));
}

#[test]
fn a_connection_timeout_is_told_apart_from_a_slow_download() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.package("word-count");
    fixture.commit("one");

    let stub = fixture.root.path().join("curl-connect-timeout");
    std::fs::write(
        &stub,
        "#!/bin/sh\ncat >/dev/null\necho 'curl: (28) Connection timed out after 30001 milliseconds' >&2\nexit 28\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut installer = fixture.without_git("connect");
    installer.github.curl = stub;
    let error = install(&installer, SPEC).unwrap_err();
    assert_eq!(error.code, "download-failed");
    assert!(error.message.contains("couldn't connect"), "{error}");
    assert!(error.hint.is_some());

    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut installer = fixture.without_git("slow");
    installer.github.api = format!("http://{}", silent.local_addr().unwrap());
    installer.github.timeout = Duration::from_secs(1);
    let error = install(&installer, SPEC).unwrap_err();
    assert_eq!(error.code, "download-failed");
    assert!(error.message.contains("took longer than 1 seconds"), "{error}");
    drop(silent);
}

const SEARCH_ALL: &str = "/search/repositories?q=topic%3Adupin-extension&sort=stars&order=desc&per_page=20";

/// A [`GitHub`] whose API and raw hosts are one loopback server under two host names, so the
/// token's scope can be checked.
fn searcher(server: &Server, token: Option<&str>) -> GitHub {
    GitHub {
        api: server.origin(),
        raw: format!("http://localhost:{}", server.port),
        timeout: Duration::from_secs(30),
        token: token.map(str::to_owned),
        ..GitHub::default()
    }
}

fn recorded(name: &str) -> Response {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/search")
        .join(name);
    Response {
        status: 200,
        location: None,
        body: std::fs::read(path).unwrap(),
    }
}

#[test]
fn search_lists_tagged_repositories_with_what_they_request_as_plain_text() {
    let server = Server::start();
    server.set(SEARCH_ALL, recorded("repositories.json"));
    server.set(
        "/someone/dupin-word-count/main/package.json",
        json_response(&json!({
            "name": "word-count",
            "version": "1.2.0",
            "dupin": {
                "title": "Word\u{202e} Count\u{2066}\u{200b}",
                "keyword": "wc",
                "capabilities": {
                    "clipboard": ["write"],
                    "http": [{"origin": "https://api.example.com", "methods": ["GET"]}]
                }
            }
        })),
    );
    server.set(
        "/mallory/dupin-escapes/main/package.json",
        json_response(&json!({
            "name": "escapes",
            "version": "1.0.0",
            "dupin": {"title": "T", "keyword": "t", "\u{1b}]0;owned\u{7}": true}
        })),
    );
    let found = searcher(&server, Some("ghp_secret")).search(None).unwrap();
    assert_eq!(found.total, 31);
    assert!(!found.incomplete);
    let names: Vec<_> = found.repositories.iter().map(|repo| repo.source()).collect();
    assert_eq!(
        names,
        [
            "github:someone/dupin-word-count",
            "github:other/dupin-unbuilt",
            "github:mallory/dupin-escapes"
        ],
        "an untagged repository and a name install refuses are left out"
    );

    let counted = &found.repositories[0];
    assert_eq!(
        counted.install_command(),
        "dupin ext install github:someone/dupin-word-count"
    );
    assert_eq!(
        counted.description.as_deref(),
        Some("Counts words[2J]8;;https://evil.example in the selection fast")
    );
    assert_eq!(
        (counted.stars, counted.pushed.as_deref()),
        (42, Some("2026-10-01"))
    );
    let package = counted.package.as_ref().unwrap();
    assert_eq!(
        (package.title.as_str(), package.keyword.as_str()),
        ("Word Count", "wc")
    );
    assert_eq!(
        package.capabilities,
        ["clipboard:write", "http:https://api.example.com"]
    );

    let unbuilt = &found.repositories[1];
    assert_eq!(unbuilt.description, None);
    assert_eq!(unbuilt.package, Err("no package.json on `trunk`".to_owned()));

    let escapes = &found.repositories[2];
    assert_eq!(escapes.pushed, None);
    let problem = escapes.package.as_ref().unwrap_err();
    assert!(problem.starts_with("not a Dupin package"), "{problem}");
    for text in [problem.as_str(), escapes.description.as_deref().unwrap()] {
        assert!(!text.chars().any(char::is_control), "{text:?}");
    }

    let seen = server.seen();
    let api_host = format!("127.0.0.1:{}", server.port);
    assert!(!seen.is_empty());
    for request in seen {
        let sent = request.authorization.is_some();
        assert_eq!(sent, request.host == api_host, "{request:?}");
        if sent {
            assert_eq!(request.authorization.as_deref(), Some("Bearer ghp_secret"));
        }
    }
}

#[test]
fn a_query_narrows_the_topic_search_and_bad_answers_say_what_to_do() {
    let server = Server::start();
    let narrowed =
        "/search/repositories?q=topic%3Adupin-extension%20clip%20board&sort=stars&order=desc&per_page=20";
    server.set(
        narrowed,
        json_response(&json!({"total_count": 0, "incomplete_results": true, "items": []})),
    );
    let github = searcher(&server, None);
    let found = github.search(Some("  clip board ")).unwrap();
    assert!(found.repositories.is_empty() && found.incomplete);

    let refused = |status: u16| Response {
        status,
        location: None,
        body: br#"{"message":"API rate limit exceeded"}"#.to_vec(),
    };
    server.set(SEARCH_ALL, refused(403));
    let error = github.search(None).unwrap_err();
    assert_eq!(error.code, "rate-limited");
    assert!(error.hint.unwrap().contains("GITHUB_TOKEN"));
    server.set(SEARCH_ALL, refused(422));
    assert_eq!(github.search(Some("")).unwrap_err().code, "invalid-query");
    server.set(SEARCH_ALL, json_response(&json!({"items": "not a list"})));
    assert_eq!(github.search(None).unwrap_err().code, "download-failed");

    let requests = server.seen().len();
    assert_eq!(
        github.search(Some("a\u{1b}[2J")).unwrap_err().code,
        "invalid-query"
    );
    assert_eq!(
        github.search(Some(&"x".repeat(201))).unwrap_err().code,
        "invalid-query"
    );
    assert_eq!(server.seen().len(), requests, "a refused query sends nothing");
}

#[test]
fn an_oversized_package_json_is_not_read() {
    let server = Server::start();
    server.set(
        SEARCH_ALL,
        json_response(&json!({
            "total_count": 1,
            "items": [{
                "name": "huge",
                "owner": {"login": "someone"},
                "default_branch": "main",
                "topics": ["dupin-extension"]
            }]
        })),
    );
    server.set(
        "/someone/huge/main/package.json",
        Response {
            status: 200,
            location: None,
            body: vec![b' '; 65 * 1024],
        },
    );
    let found = searcher(&server, None).search(None).unwrap();
    assert_eq!(
        found.repositories[0].package,
        Err("package.json is larger than 64 KiB".to_owned())
    );
}
