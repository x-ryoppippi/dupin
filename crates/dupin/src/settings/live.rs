use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};

use dupin_core::Config;
use dupin_mac::{PathWatcher, watcher::canonical_root};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub revision: u64,
    pub read_only: bool,
    pub result: Result<Config, String>,
    pub watch_problem: Option<String>,
}

pub struct Service {
    path: Option<PathBuf>,
    sender: Sender<Job>,
    next_revision: Mutex<u64>,
    worker: Option<JoinHandle<()>>,
}

impl Service {
    pub fn new(path: Option<PathBuf>, callback: impl Fn(Update) + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker_sender = sender.clone();
        let worker_path = path.clone();
        let worker = thread::Builder::new()
            .name("dupin-config-watch".into())
            .spawn(move || Worker::new(worker_path, receiver, worker_sender, callback).run())
            .expect("could not start settings worker");

        Service {
            path,
            sender,
            next_revision: Mutex::new(0),
            worker: Some(worker),
        }
    }

    pub fn activate(&self) {
        let _ = self.sender.send(Job::Activate);
    }

    pub fn revision(&self) -> u64 {
        *self.next_revision.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn save(&self, config: &Config) -> Result<u64, String> {
        if self.path.is_none() {
            return Err("config.toml has no path".into());
        }

        let mut next = self.next_revision.lock().unwrap_or_else(|e| e.into_inner());
        let revision = next
            .checked_add(1)
            .ok_or_else(|| "settings revision is exhausted".to_owned())?;
        self.sender
            .send(Job::Save {
                revision,
                config: config.clone(),
            })
            .map_err(|_| "settings worker stopped".to_owned())?;
        *next = revision;
        Ok(revision)
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.sender.send(Job::Stop);
        let worker = self.worker.take();
        if let Some(worker) = worker
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
    }
}

enum Job {
    Save { revision: u64, config: Config },
    Changed,
    Activate,
    Stop,
}

#[derive(Default)]
struct Batch {
    save: Option<(u64, Config)>,
    changed: bool,
    activate: bool,
    stop: bool,
}

impl Batch {
    fn collect(first: Job, receiver: &Receiver<Job>) -> Batch {
        let mut batch = Batch::default();
        batch.push(first);
        loop {
            match receiver.try_recv() {
                Ok(job) => batch.push(job),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return batch,
            }
        }
    }

    fn push(&mut self, job: Job) {
        match job {
            Job::Save { revision, config } => {
                if self.save.as_ref().is_none_or(|(current, _)| revision > *current) {
                    self.save = Some((revision, config));
                }
            }
            Job::Changed => self.changed = true,
            Job::Activate => self.activate = true,
            Job::Stop => self.stop = true,
        }
    }
}

struct Worker<F> {
    path: Option<PathBuf>,
    receiver: Receiver<Job>,
    sender: Sender<Job>,
    callback: F,
    watcher: Option<PathWatcher>,
    watcher_roots: Option<Vec<PathBuf>>,
    watch_problem: Option<String>,
    last_processed_revision: u64,
    last_emitted: Option<Emitted>,
    activated: bool,
}

impl<F> Worker<F>
where
    F: Fn(Update),
{
    fn new(path: Option<PathBuf>, receiver: Receiver<Job>, sender: Sender<Job>, callback: F) -> Self {
        Worker {
            path,
            receiver,
            sender,
            callback,
            watcher: None,
            watcher_roots: None,
            watch_problem: None,
            last_processed_revision: 0,
            last_emitted: None,
            activated: false,
        }
    }

    fn run(mut self) {
        while let Ok(first) = self.receiver.recv() {
            let batch = Batch::collect(first, &self.receiver);
            if batch.stop {
                break;
            }

            if batch.activate {
                self.activated = true;
            }
            if self.activated && (batch.activate || batch.changed || batch.save.is_some()) {
                self.refresh_watcher();
            }

            if let Some((revision, config)) = batch.save {
                self.process_save(revision, config);
            } else if batch.activate || batch.changed {
                self.emit_snapshot(self.last_processed_revision);
            }
        }
        self.watcher.take();
    }

    fn process_save(&mut self, revision: u64, config: Config) {
        if revision <= self.last_processed_revision {
            return;
        }
        self.last_processed_revision = revision;

        let Some(path) = &self.path else {
            return;
        };
        let saved = toml::to_string(&config)
            .map_err(|_| "could not encode config.toml".to_owned())
            .and_then(|source| {
                super::write(path, &source).map_err(|error| format!("could not save config.toml: {error}"))
            });

        match saved {
            Ok(()) => self.emit_snapshot(revision),
            Err(error) => self.emit(None, Err(error), self.read_only(), revision),
        }
    }

    fn emit_snapshot(&mut self, revision: u64) {
        let (source, result, read_only) = match &self.path {
            Some(path) => read(path),
            None => (None, Ok(Config::default()), false),
        };
        self.emit(source, result, read_only, revision);
    }

    fn emit(
        &mut self,
        source: Option<String>,
        result: Result<Config, String>,
        read_only: bool,
        revision: u64,
    ) {
        let update = Update {
            revision,
            read_only,
            result,
            watch_problem: self.watch_problem.clone(),
        };
        let emitted = Emitted {
            source,
            update: update.clone(),
        };
        if self.last_emitted.as_ref() == Some(&emitted) {
            return;
        }
        self.last_emitted = Some(emitted);
        (self.callback)(update);
    }

    fn read_only(&self) -> bool {
        self.path.as_deref().is_some_and(|path| !super::writable(path))
    }

    fn refresh_watcher(&mut self) {
        let Some(path) = &self.path else {
            self.watcher.take();
            self.watcher_roots = None;
            self.watch_problem = None;
            return;
        };

        let roots = roots(path);
        if self.watcher.is_some() && self.watcher_roots.as_ref() == Some(&roots.paths) {
            self.watch_problem = roots.problem;
            return;
        }

        let sender = self.sender.clone();
        match PathWatcher::start(roots.paths.clone(), move || {
            let _ = sender.send(Job::Changed);
        }) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.watcher_roots = Some(roots.paths);
                self.watch_problem = roots.problem;
            }
            Err(error) => {
                self.watch_problem = Some(match roots.problem {
                    Some(problem) => format!("{problem}; could not watch config.toml: {error}"),
                    None => format!("could not watch config.toml: {error}"),
                });
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Emitted {
    source: Option<String>,
    update: Update,
}

struct WatchRoots {
    paths: Vec<PathBuf>,
    problem: Option<String>,
}

fn roots(path: &Path) -> WatchRoots {
    let mut problem = None;
    let configured = match absolute(path) {
        Ok(path) => path,
        Err(error) => {
            problem = Some(format!("could not locate config.toml ({:?})", error.kind()));
            path.to_owned()
        }
    };
    let mut paths = Vec::new();
    if let Err(error) = super::resolve_with(&configured, |visited| {
        add_parent(visited, &mut paths);
        add_symlink_parents(visited, &mut paths);
    }) {
        problem.get_or_insert_with(|| format!("could not resolve config.toml ({:?})", error.kind()));
    }

    let mut paths: Vec<_> = paths.into_iter().map(|path| canonical_root(&path)).collect();
    paths.sort();
    paths.dedup();
    WatchRoots { paths, problem }
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn add_parent(path: &Path, roots: &mut Vec<PathBuf>) {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        roots.push(parent.to_owned());
    }
}

fn add_symlink_parents(path: &Path, roots: &mut Vec<PathBuf>) {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix_component) => prefix.push(prefix_component.as_os_str()),
            Component::RootDir => prefix.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => prefix.push(".."),
            Component::Normal(name) => prefix.push(name),
        }
        if fs::symlink_metadata(&prefix).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            add_parent(&prefix, roots);
        }
    }
}

fn read(path: &Path) -> (Option<String>, Result<Config, String>, bool) {
    let read_only = !super::writable(path);
    match fs::read_to_string(path) {
        Ok(source) => {
            let result = Config::parse(&source).map_err(|error| parse_problem(&source, &error));
            (Some(source), result, read_only)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => (None, Ok(Config::default()), read_only),
        Err(error) => (
            None,
            Err(format!("could not read config.toml ({:?})", error.kind())),
            read_only,
        ),
    }
}

fn parse_problem(source: &str, error: &toml::de::Error) -> String {
    let Some(span) = error.span() else {
        return "config.toml is invalid (location unavailable)".into();
    };
    let mut start = span.start.min(source.len());
    while !source.is_char_boundary(start) {
        start -= 1;
    }
    let prefix = &source[..start];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map_or(1, |line| line.chars().count() + 1);
    format!("config.toml is invalid at line {line}, column {column}")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> TempDir {
            loop {
                let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
                let path =
                    std::env::temp_dir().join(format!("dupin-live-settings-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return TempDir(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("could not create test directory: {error}"),
                }
            }
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn send_changed(service: &Service) {
        service.sender.send(Job::Changed).unwrap();
    }

    fn receive_revision(receiver: &Receiver<Update>, revision: u64) -> Update {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let update = receiver.recv_timeout(remaining).unwrap();
            if update.revision == revision {
                return update;
            }
        }
    }

    #[test]
    fn queued_saves_are_coalesced_to_the_highest_revision() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Job::Save {
                revision: 1,
                config: Config {
                    file_limit: 50,
                    ..Config::default()
                },
            })
            .unwrap();
        sender
            .send(Job::Save {
                revision: 2,
                config: Config {
                    file_limit: 500,
                    ..Config::default()
                },
            })
            .unwrap();
        sender.send(Job::Changed).unwrap();

        let batch = Batch::collect(receiver.recv().unwrap(), &receiver);
        let (revision, config) = batch.save.unwrap();
        assert_eq!(revision, 2);
        assert_eq!(config.file_limit, 500);
        assert!(batch.changed);
    }

    #[test]
    fn rapid_saves_finish_at_the_latest_config_and_tag_the_read() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(Some(path.clone()), move |update| updates.send(update).unwrap());
        let first = Config {
            file_limit: 50,
            ..Config::default()
        };
        let latest = Config {
            file_limit: 500,
            ..Config::default()
        };

        let first_revision = service.save(&first).unwrap();
        let latest_revision = service.save(&latest).unwrap();
        assert!(latest_revision > first_revision);

        let update = receive_revision(&receiver, latest_revision);
        assert_eq!(update.result.unwrap(), latest);
        assert_eq!(Config::parse(&fs::read_to_string(path).unwrap()).unwrap(), latest);
    }

    #[test]
    fn invalid_config_keeps_a_safe_location_and_recovers_on_a_later_change() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        let source = "hotkey = \"private-value\n";
        fs::write(&path, source).unwrap();
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(Some(path.clone()), move |update| updates.send(update).unwrap());

        send_changed(&service);
        let invalid = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let error = invalid.result.unwrap_err();
        assert!(error.contains("line"));
        assert!(!error.contains("private-value"));

        let expected = Config {
            position: dupin_core::Position::Top,
            ..Config::default()
        };
        fs::write(&path, toml::to_string(&expected).unwrap()).unwrap();
        send_changed(&service);
        let recovered = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(recovered.result.unwrap(), expected);
        assert_eq!(recovered.revision, invalid.revision);
    }

    #[test]
    fn deleting_the_file_restores_defaults() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        fs::write(&path, "file_limit = 50").unwrap();
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(Some(path.clone()), move |update| updates.send(update).unwrap());

        send_changed(&service);
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .result
                .unwrap()
                .file_limit,
            50
        );
        fs::remove_file(&path).unwrap();
        send_changed(&service);
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .result
                .unwrap(),
            Config::default()
        );
    }

    #[test]
    fn roots_follow_file_and_directory_symlink_retargets() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let home = temp.path().join("home");
        let generation_one = temp.path().join("generation-one");
        let generation_two = temp.path().join("generation-two");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(generation_one.join("dupin")).unwrap();
        fs::create_dir_all(generation_two.join("dupin")).unwrap();
        let config_dir = home.join(".config");
        symlink(&generation_one, &config_dir).unwrap();
        let path = config_dir.join("dupin/config.toml");

        let first = roots(&path);
        assert!(first.paths.contains(&canonical_root(&home)));
        assert!(
            first
                .paths
                .contains(&canonical_root(&generation_one.join("dupin")))
        );

        fs::remove_file(&config_dir).unwrap();
        symlink(&generation_two, &config_dir).unwrap();
        let second = roots(&path);
        assert!(second.paths.contains(&canonical_root(&home)));
        assert!(
            second
                .paths
                .contains(&canonical_root(&generation_two.join("dupin")))
        );
        assert!(
            !second
                .paths
                .contains(&canonical_root(&generation_one.join("dupin")))
        );

        let external_one = temp.path().join("external-one");
        let external_two = temp.path().join("external-two");
        fs::create_dir(&external_one).unwrap();
        fs::create_dir(&external_two).unwrap();
        let file_target = external_one.join("config.toml");
        fs::write(&file_target, "").unwrap();
        let file_link = temp.path().join("linked.toml");
        symlink(&file_target, &file_link).unwrap();
        let linked_roots = roots(&file_link);
        assert!(linked_roots.paths.contains(&canonical_root(temp.path())));
        assert!(linked_roots.paths.contains(&canonical_root(&external_one)));

        fs::remove_file(&file_link).unwrap();
        let next_target = external_two.join("config.toml");
        fs::write(&next_target, "").unwrap();
        symlink(&next_target, &file_link).unwrap();
        let repointed_roots = roots(&file_link);
        assert!(repointed_roots.paths.contains(&canonical_root(&external_two)));
        assert!(!repointed_roots.paths.contains(&canonical_root(&external_one)));
    }

    #[test]
    fn real_watcher_reloads_changes_and_stops_with_the_service() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        fs::write(&path, "position = \"center\"").unwrap();
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(Some(path.clone()), move |update| updates.send(update).unwrap());

        service.activate();
        let initial = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(initial.result.unwrap().position, dupin_core::Position::Center);

        fs::write(&path, "position = \"top\"").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let changed = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let update = receiver.recv_timeout(remaining).unwrap();
            if update
                .result
                .as_ref()
                .is_ok_and(|config| config.position == dupin_core::Position::Top)
            {
                break update;
            }
        };
        assert_eq!(changed.revision, 0);

        let (done, stopped) = mpsc::channel();
        thread::spawn(move || {
            drop(service);
            let _ = done.send(());
        });
        stopped.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
