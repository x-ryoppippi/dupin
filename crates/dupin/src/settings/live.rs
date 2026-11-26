use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};

use dupin_core::Config;
use dupin_mac::{PathWatcher, watcher::canonical_root};

use super::layers::Layers;
use super::{LockedKeys, Paths};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub revision: u64,
    pub locked_keys: Option<LockedKeys>,
    pub result: Result<Config, String>,
    pub watch_problem: Option<String>,
    pub save_problem: Option<String>,
    pub save_notice: Option<String>,
}

pub struct Service {
    base: Option<PathBuf>,
    sender: Sender<Job>,
    next_revision: Mutex<u64>,
    worker: Option<JoinHandle<()>>,
}

impl Service {
    pub fn new(paths: Paths, callback: impl Fn(Update) + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker_sender = sender.clone();
        let base = paths.base.clone();
        let worker = thread::Builder::new()
            .name("dupin-config-watch".into())
            .spawn(move || Worker::new(paths, receiver, worker_sender, callback).run())
            .expect("could not start settings worker");

        Service {
            base,
            sender,
            next_revision: Mutex::new(0),
            worker: Some(worker),
        }
    }

    pub(crate) fn base_path(&self) -> Option<&Path> {
        self.base.as_deref()
    }

    pub fn activate(&self) {
        let _ = self.sender.send(Job::Activate);
    }

    /// Reads the layers again, as a change on disk would make it.
    pub fn reread(&self) {
        let _ = self.sender.send(Job::Changed { rebuild: false });
    }

    pub fn revision(&self) -> u64 {
        *self.next_revision.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn save(&self, base: &Config, desired: &Config) -> Result<u64, String> {
        if self.base.is_none() {
            return Err("config.toml has no path".into());
        }

        let mut next = self.next_revision.lock().unwrap_or_else(|e| e.into_inner());
        let revision = next
            .checked_add(1)
            .ok_or_else(|| "settings revision is exhausted".to_owned())?;
        self.sender
            .send(Job::Save(Box::new(Save {
                revision,
                base: base.clone(),
                desired: desired.clone(),
            })))
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
    Save(Box<Save>),
    Changed { rebuild: bool },
    Activate,
    Stop,
}

#[derive(Default)]
struct Batch {
    saves: Vec<Save>,
    changed: bool,
    rebuild: bool,
    activate: bool,
    stop: bool,
}

struct Save {
    revision: u64,
    base: Config,
    desired: Config,
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
            Job::Save(save) => self.saves.push(*save),
            Job::Changed { rebuild } => {
                self.changed = true;
                self.rebuild |= rebuild;
            }
            Job::Activate => self.activate = true,
            Job::Stop => self.stop = true,
        }
    }
}

struct Worker<F> {
    paths: Paths,
    receiver: Receiver<Job>,
    sender: Sender<Job>,
    callback: F,
    watcher: Option<PathWatcher>,
    watcher_roots: Option<Vec<PathBuf>>,
    watch_problem: Option<String>,
    save_problem: Option<String>,
    save_notice: Option<String>,
    uncommitted: Option<(Config, Config)>,
    last_processed_revision: u64,
    last_emitted: Option<Emitted>,
    activated: bool,
    write: fn(&Path, &str) -> io::Result<()>,
}

impl<F> Worker<F>
where
    F: Fn(Update),
{
    fn new(paths: Paths, receiver: Receiver<Job>, sender: Sender<Job>, callback: F) -> Self {
        Worker {
            paths,
            receiver,
            sender,
            callback,
            watcher: None,
            watcher_roots: None,
            watch_problem: None,
            save_problem: None,
            save_notice: None,
            uncommitted: None,
            last_processed_revision: 0,
            last_emitted: None,
            activated: false,
            write: super::write,
        }
    }

    fn run(mut self) {
        while let Ok(first) = self.receiver.recv() {
            let batch = Batch::collect(first, &self.receiver);
            if batch.activate {
                self.activated = true;
            }
            if !batch.stop && self.activated && (batch.activate || batch.changed || !batch.saves.is_empty()) {
                self.refresh_watcher(batch.rebuild);
            }

            if !batch.saves.is_empty() {
                self.process_saves(batch.saves);
            } else if !batch.stop && (batch.activate || batch.changed) {
                self.emit_snapshot(self.last_processed_revision);
            }

            if batch.stop {
                break;
            }
        }
        self.watcher.take();
    }

    fn process_saves(&mut self, saves: Vec<Save>) {
        let saves: Vec<_> = saves
            .into_iter()
            .filter(|save| save.revision > self.last_processed_revision)
            .collect();
        let Some(latest_revision) = saves.last().map(|save| save.revision) else {
            return;
        };
        self.last_processed_revision = latest_revision;

        let original_base = self
            .uncommitted
            .take()
            .filter(|(_, desired)| saves[0].base == *desired)
            .map(|(base, _)| base);

        match save_to_disk(&self.paths, &saves, original_base.as_ref(), self.write) {
            Ok(notice) => {
                self.save_problem = None;
                self.save_notice = notice;
            }
            Err(error) => {
                self.save_problem = Some(error);
                self.uncommitted = Some((
                    original_base.unwrap_or_else(|| saves[0].base.clone()),
                    saves.last().unwrap().desired.clone(),
                ));
            }
        }
        self.emit_snapshot(latest_revision);
    }

    fn emit_snapshot(&mut self, revision: u64) {
        let layers = self.paths.read();
        let (result, locked_keys) = match layers.as_ref().map_err(String::clone).and_then(Layers::effective) {
            Ok(effective) => (Ok(effective.config), Some(effective.locked_keys)),
            Err(error) => (Err(error), None),
        };
        let update = Update {
            revision,
            locked_keys,
            result,
            watch_problem: self.watch_problem.clone(),
            save_problem: self.save_problem.clone(),
            save_notice: self.save_notice.clone(),
        };
        let emitted = Emitted {
            layers,
            update: update.clone(),
        };
        if self.last_emitted.as_ref() == Some(&emitted) {
            return;
        }
        self.last_emitted = Some(emitted);
        (self.callback)(update);
    }

    fn refresh_watcher(&mut self, rebuild: bool) {
        let files = self.paths.base.iter();
        if files.clone().next().is_none() {
            self.watcher.take();
            self.watcher_roots = None;
            self.watch_problem = None;
            return;
        }

        let requested = roots(files.clone().map(PathBuf::as_path));
        if !rebuild && self.watcher.is_some() && self.watcher_roots.as_ref() == Some(&requested.paths) {
            self.watch_problem = requested.problem;
            return;
        }

        let sender = self.sender.clone();
        match PathWatcher::start_shallow_with_files(
            requested.paths.clone(),
            files.clone().cloned().collect(),
            move |rebuild| {
                let _ = sender.send(Job::Changed { rebuild });
            },
        ) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                if roots(files.map(PathBuf::as_path)).paths != requested.paths {
                    let _ = self.sender.send(Job::Changed { rebuild: true });
                }
                self.watcher_roots = Some(requested.paths);
                self.watch_problem = requested.problem;
            }
            Err(error) => {
                self.watcher_roots = None;
                self.watch_problem = Some(match requested.problem {
                    Some(problem) => format!("{problem}; could not watch settings files: {error}"),
                    None => format!("could not watch settings files: {error}"),
                });
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Emitted {
    layers: Result<Layers, String>,
    update: Update,
}

struct WatchRoots {
    paths: Vec<PathBuf>,
    problem: Option<String>,
}

fn roots<'a>(files: impl IntoIterator<Item = &'a Path>) -> WatchRoots {
    let mut problem = None;
    let mut paths = Vec::new();
    for path in files {
        let configured = match absolute(path) {
            Ok(path) => path,
            Err(error) => {
                problem = Some(format!(
                    "could not locate {} ({:?})",
                    path.display(),
                    error.kind()
                ));
                path.to_owned()
            }
        };
        if let Err(error) = super::resolve_with(&configured, |visited| add_parent(visited, &mut paths)) {
            problem
                .get_or_insert_with(|| format!("could not resolve {} ({:?})", path.display(), error.kind()));
        }
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

fn save_to_disk(
    paths: &Paths,
    saves: &[Save],
    original_base: Option<&Config>,
    write: fn(&Path, &str) -> io::Result<()>,
) -> Result<Option<String>, String> {
    let layers = paths.read()?;
    let effective = layers.effective()?;
    let mut current = effective.config.clone();
    for (index, save) in saves.iter().enumerate() {
        current = merge(
            &save.base,
            &save.desired,
            &current,
            original_base.filter(|_| index == 0),
            &effective.locked_keys,
        )?;
    }
    if layers.base_read_only {
        let stored = paths
            .overlay
            .as_deref()
            .ok_or("Dupin's storage needs XDG_DATA_HOME or HOME")?;
        let mut overlay = effective.overlay;
        let mut dropped = Vec::new();
        for key in &effective.locked_keys {
            if overlay.remove(key).is_some() {
                dropped.push(key.as_str());
            }
        }
        let before = toml::Table::try_from(&effective.config).map_err(|_| "could not encode the settings")?;
        let after = toml::Table::try_from(&current).map_err(|_| "could not encode the settings")?;
        overlay.extend(
            after
                .into_iter()
                .filter(|(key, value)| before.get(key) != Some(value)),
        );
        if overlay.is_empty() && dropped.is_empty() {
            return Ok(None);
        }
        unchanged_since(paths, &layers)?;
        stored
            .write(&overlay)
            .map_err(|error| format!("could not save the settings: {error}"))?;
        Ok((!dropped.is_empty()).then(|| {
            format!(
                "Removed saved settings that config.toml now sets: {}.",
                dropped.join(", ")
            )
        }))
    } else {
        let path = paths.base.as_deref().ok_or("config.toml has no path")?;
        let source = toml::to_string(&current).map_err(|_| "could not encode config.toml")?;
        unchanged_since(paths, &layers)?;
        write(path, &source).map_err(|error| format!("could not save {}: {error}", path.display()))?;
        Ok(None)
    }
}

fn unchanged_since(paths: &Paths, layers: &Layers) -> Result<(), String> {
    if paths.read().as_ref() != Ok(layers) {
        return Err("configuration changed on disk while saving; no changes were written".into());
    }
    Ok(())
}

pub fn check_locked(base: &Config, desired: &Config, locked: &LockedKeys) -> Result<(), String> {
    if locked.is_empty() {
        return Ok(());
    }
    let base = config_value(base)?;
    let desired = config_value(desired)?;
    reject_locked_changes(&base, &desired, locked)
}

fn config_value(config: &Config) -> Result<serde_json::Value, String> {
    serde_json::to_value(config).map_err(|_| "could not compare config.toml".to_owned())
}

fn reject_locked_changes(
    base: &serde_json::Value,
    desired: &serde_json::Value,
    locked: &LockedKeys,
) -> Result<(), String> {
    if let Some(key) = locked.iter().find(|key| base.get(*key) != desired.get(*key)) {
        return Err(format!(
            "'{key}' is set in read-only config.toml. Change it where config.toml is managed."
        ));
    }
    Ok(())
}

fn merge(
    base: &Config,
    desired: &Config,
    current: &Config,
    original_base: Option<&Config>,
    locked: &LockedKeys,
) -> Result<Config, String> {
    let base = config_value(base)?;
    let desired = config_value(desired)?;
    reject_locked_changes(&base, &desired, locked)?;
    let mut merged = config_value(current)?;
    let original_base = original_base.map(config_value).transpose()?;

    let (Some(base), Some(desired), Some(merged)) =
        (base.as_object(), desired.as_object(), merged.as_object_mut())
    else {
        return Err("could not compare config.toml".into());
    };

    for (key, desired_value) in desired {
        if locked.contains(key) {
            continue;
        }
        let Some(base_value) = base.get(key) else {
            return Err("could not compare config.toml".into());
        };
        let Some(current_value) = merged.get(key) else {
            return Err("could not compare config.toml".into());
        };
        let base_value = original_base
            .as_ref()
            .and_then(|base| base.get(key))
            .filter(|original| *original == current_value)
            .unwrap_or(base_value);
        if desired_value == base_value {
            continue;
        }
        if current_value != base_value && current_value != desired_value {
            return Err(
                "configuration changed on disk; resolve the conflicting setting before saving".into(),
            );
        }
        merged.insert(key.clone(), desired_value.clone());
    }

    serde_json::from_value(serde_json::Value::Object(merged.clone()))
        .map_err(|_| "could not merge config.toml".into())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::settings::overlay::Overlay;

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

    fn base_only(path: PathBuf) -> Paths {
        Paths {
            base: Some(path),
            overlay: None,
        }
    }

    fn linked_base(temp: &TempDir, source: &str) -> (Paths, PathBuf, std::sync::Arc<Overlay>) {
        let target = temp.path().join("managed.toml");
        fs::write(&target, source).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o444)).unwrap();
        let base = temp.path().join("config.toml");
        symlink(&target, &base).unwrap();
        let storage = dupin_storage::Storage::new(
            temp.path().join("data/storage"),
            dupin_storage::MemoryKey::random(),
        );
        let overlay = std::sync::Arc::new(Overlay::new(std::sync::Arc::new(storage)));
        (
            Paths {
                base: Some(base),
                overlay: Some(overlay.clone()),
            },
            target,
            overlay,
        )
    }

    fn stored(overlay: &Overlay) -> Option<toml::Table> {
        overlay.read().unwrap().map(|source| source.parse().unwrap())
    }

    #[test]
    fn locked_keys_take_precedence_as_whole_values_and_only_unlocked_edits_are_saved() {
        let temp = TempDir::new();
        let base_source = "hotkey = \"alt+space\"\n[theme]\ncorner_radius = 28\n";
        let (paths, target, overlay) = linked_base(&temp, base_source);
        let link = paths.base.clone().unwrap();
        let (updates, receiver) = mpsc::channel();
        let (base, locked, problem) = super::super::load(&paths);
        assert_eq!(problem, None);
        assert_eq!(locked, ["hotkey".into(), "theme".into()].into());
        assert_eq!(base.theme.corner_radius, 28);
        assert_eq!(base.file_limit, Config::default().file_limit);
        let service = Service::new(paths, move |update| updates.send(update).unwrap());

        let edit = Config {
            file_limit: 123,
            ..base.clone()
        };
        let saved = receive_revision(&receiver, service.save(&base, &edit).unwrap());
        assert_eq!(saved.result.unwrap(), edit);
        assert!(saved.save_problem.is_none());
        assert_eq!(stored(&overlay), Some("file_limit = 123".parse().unwrap()));
        assert!(!temp.path().join("data/settings.toml").exists());

        let shadowed: toml::Table = "file_limit = 123\nhotkey = \"ctrl+alt+k\"\n[theme]\npanel_width = 900\n"
            .parse()
            .unwrap();
        overlay.write(&shadowed).unwrap();
        let attempt = Config {
            hotkey: "ctrl+alt+k".into(),
            show_recent: false,
            ..edit.clone()
        };
        let rejected = receive_revision(&receiver, service.save(&edit, &attempt).unwrap());
        assert_eq!(rejected.result.unwrap(), edit);
        assert!(
            rejected
                .save_problem
                .unwrap()
                .contains("'hotkey' is set in read-only config.toml")
        );
        assert_eq!(stored(&overlay), Some(shadowed));

        let next = Config {
            show_recent: false,
            ..edit.clone()
        };
        let saved = receive_revision(&receiver, service.save(&edit, &next).unwrap());
        assert_eq!(saved.result.unwrap(), next);
        assert!(saved.save_problem.is_none());
        assert!(saved.save_notice.unwrap().contains("hotkey, theme"));
        assert_eq!(
            stored(&overlay).unwrap().keys().collect::<Vec<_>>(),
            vec!["file_limit", "show_recent"]
        );
        assert_eq!(fs::read_to_string(target).unwrap(), base_source);
        assert!(fs::symlink_metadata(link).unwrap().is_symlink());
    }

    #[test]
    fn overlay_saves_merge_what_another_writer_stored_and_reject_conflicts() {
        let temp = TempDir::new();
        let (paths, _, overlay) = linked_base(&temp, "position = \"top\"");
        let (base, _, _) = super::super::load(&paths);
        let elsewhere: toml::Table = "file_limit = 71".parse().unwrap();
        overlay.write(&elsewhere).unwrap();
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(paths, move |update| updates.send(update).unwrap());

        let conflicting = Config {
            file_limit: 123,
            ..base.clone()
        };
        let rejected = receive_revision(&receiver, service.save(&base, &conflicting).unwrap());
        assert!(rejected.save_problem.unwrap().contains("conflicting setting"));
        assert_eq!(stored(&overlay), Some(elsewhere));

        let desired = Config {
            show_recent: false,
            ..base.clone()
        };
        let saved = receive_revision(&receiver, service.save(&base, &desired).unwrap());
        assert!(saved.save_problem.is_none());
        assert_eq!(
            saved.result.unwrap(),
            Config {
                file_limit: 71,
                ..desired
            }
        );
    }

    #[test]
    fn writable_base_saves_through_its_link_without_creating_an_overlay() {
        let temp = TempDir::new();
        let (paths, target, overlay) = linked_base(&temp, "file_limit = 71");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        let (base, locked, _) = super::super::load(&paths);
        assert!(locked.is_empty());
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(paths, move |update| updates.send(update).unwrap());
        let desired = Config {
            file_limit: 123,
            ..base.clone()
        };
        let saved = receive_revision(&receiver, service.save(&base, &desired).unwrap());
        assert_eq!(saved.result.unwrap(), desired);
        assert_eq!(
            Config::parse(&fs::read_to_string(target).unwrap()).unwrap(),
            desired
        );
        assert_eq!(stored(&overlay), None);
        assert!(!temp.path().join("data/storage").exists(), "no key read");
    }

    #[test]
    fn an_overlay_that_cant_be_read_refuses_saves_and_keeps_what_it_holds() {
        let temp = TempDir::new();
        let (paths, _, overlay) = linked_base(&temp, "position = \"top\"");
        overlay.write(&"file_limit = 71".parse().unwrap()).unwrap();
        drop(paths);
        drop(overlay);
        let locked = Paths {
            base: Some(temp.path().join("config.toml")),
            overlay: Some(std::sync::Arc::new(Overlay::new(std::sync::Arc::new(
                dupin_storage::Storage::new(
                    temp.path().join("data/storage"),
                    dupin_storage::MemoryKey::random(),
                ),
            )))),
        };
        let (_, _, problem) = super::super::load(&locked);
        assert!(problem.unwrap().contains("could not read the stored settings"));
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(locked, move |update| updates.send(update).unwrap());
        let desired = Config {
            show_recent: false,
            ..Config::default()
        };
        let refused = receive_revision(&receiver, service.save(&Config::default(), &desired).unwrap());
        assert!(refused.result.is_err());
        assert!(
            refused
                .save_problem
                .unwrap()
                .contains("could not read the stored settings")
        );
    }

    fn send_changed(service: &Service) {
        service.sender.send(Job::Changed { rebuild: false }).unwrap();
    }

    fn save_job(revision: u64, base: Config, desired: Config) -> Job {
        Job::Save(Box::new(Save {
            revision,
            base,
            desired,
        }))
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
    fn reload_and_settings_save_keep_a_missing_script_binding() {
        let root = TempDir::new();
        let config_path = root.path().join("config.toml");
        let source = r#"
file_limit = 37
[bindings."alt+cmd+t"]
target = "script"
script = "missing.sh"
"#;
        fs::write(&config_path, source).unwrap();
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(base_only(config_path.clone()), move |update| {
            updates.send(update).unwrap()
        });
        send_changed(&service);
        let config = Config::parse(source).unwrap();
        assert_eq!(receive_revision(&receiver, 0).result.unwrap(), config);

        let desired = Config {
            file_limit: 58,
            ..config.clone()
        };
        let revision = service.save(&config, &desired).unwrap();
        let saved = receive_revision(&receiver, revision);
        assert!(saved.save_problem.is_none());
        assert_eq!(saved.result.unwrap(), desired);
        assert_eq!(
            Config::parse(&fs::read_to_string(config_path).unwrap()).unwrap(),
            desired
        );
    }

    #[test]
    fn queued_saves_keep_order_for_a_single_merged_write() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(save_job(
                1,
                Config::default(),
                Config {
                    file_limit: 50,
                    ..Config::default()
                },
            ))
            .unwrap();
        sender
            .send(save_job(
                2,
                Config {
                    file_limit: 50,
                    ..Config::default()
                },
                Config {
                    file_limit: 500,
                    position: dupin_core::Position::Top,
                    ..Config::default()
                },
            ))
            .unwrap();
        sender.send(Job::Changed { rebuild: false }).unwrap();

        let batch = Batch::collect(receiver.recv().unwrap(), &receiver);
        assert_eq!(batch.saves.len(), 2);
        assert_eq!(batch.saves[0].revision, 1);
        assert_eq!(batch.saves[1].revision, 2);
        assert!(batch.changed);
    }

    #[test]
    fn theme_saves_preserve_other_settings_and_detect_conflicting_themes() {
        let base = Config::default();
        let mut desired = base.clone();
        desired.theme.panel_width = 400;
        let mut disk = base.clone();
        disk.file_limit = 42;
        let saved = merge(&base, &desired, &disk, None, &LockedKeys::new()).unwrap();
        assert_eq!(saved.theme.panel_width, 400);
        assert_eq!(saved.file_limit, 42);
        disk.theme.row_height = 72;
        assert!(merge(&base, &desired, &disk, None, &LockedKeys::new()).is_err());
        assert_eq!(
            merge(&base, &base, &disk, None, &LockedKeys::new()).unwrap(),
            disk
        );
    }

    #[test]
    fn worker_persists_the_latest_queued_save_before_stopping() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        let (sender, receiver) = mpsc::channel();
        let base = Config::default();
        let first = Config {
            file_limit: 50,
            ..base.clone()
        };
        sender.send(save_job(1, base, first.clone())).unwrap();
        let latest = Config {
            file_limit: 500,
            position: dupin_core::Position::Top,
            ..Config::default()
        };
        sender.send(save_job(2, first, latest.clone())).unwrap();
        sender.send(Job::Stop).unwrap();

        Worker::new(base_only(path.clone()), receiver, sender, |_| {}).run();

        assert_eq!(Config::parse(&fs::read_to_string(path).unwrap()).unwrap(), latest);
    }

    #[test]
    fn rapid_saves_preserve_changes_to_different_fields_and_tag_the_read() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(base_only(path.clone()), move |update| {
            updates.send(update).unwrap()
        });
        let base = Config::default();
        let first = Config {
            file_limit: 50,
            ..base.clone()
        };
        let latest = Config {
            position: dupin_core::Position::Top,
            ..first.clone()
        };

        let first_revision = service.save(&base, &first).unwrap();
        let latest_revision = service.save(&first, &latest).unwrap();
        assert!(latest_revision > first_revision);

        let update = receive_revision(&receiver, latest_revision);
        assert_eq!(update.result.unwrap(), latest);
        assert_eq!(Config::parse(&fs::read_to_string(path).unwrap()).unwrap(), latest);
    }

    #[test]
    fn external_edits_are_preserved_when_settings_change_other_fields() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        let base = Config::default();
        fs::write(&path, toml::to_string(&base).unwrap()).unwrap();

        let desired = Config {
            position: dupin_core::Position::Top,
            file_limit: 50,
            ..base.clone()
        };
        let external = Config {
            dictionary_keywords: vec!["meaning".into()],
            ..base.clone()
        };
        fs::write(&path, toml::to_string(&external).unwrap()).unwrap();

        let (sender, receiver) = mpsc::channel();
        sender.send(save_job(1, base, desired.clone())).unwrap();
        sender.send(Job::Stop).unwrap();
        let (updates, update_receiver) = mpsc::channel();
        Worker::new(base_only(path.clone()), receiver, sender, move |update| {
            updates.send(update).unwrap();
        })
        .run();

        let update = update_receiver.try_recv().unwrap();
        let mut expected = desired;
        expected.dictionary_keywords = vec!["meaning".into()];
        assert_eq!(update.result.unwrap(), expected);
        assert_eq!(
            Config::parse(&fs::read_to_string(path).unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn conflicting_external_edit_leaves_the_file_unchanged() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        let base = Config::default();
        let source = "dictionary_keywords = [\"meaning\"]\n";
        fs::write(&path, source).unwrap();
        let desired = Config {
            dictionary_keywords: vec!["lookup".into()],
            position: dupin_core::Position::Top,
            ..base.clone()
        };

        let (sender, receiver) = mpsc::channel();
        sender.send(save_job(1, base, desired)).unwrap();
        sender.send(Job::Stop).unwrap();
        let (updates, update_receiver) = mpsc::channel();
        Worker::new(base_only(path.clone()), receiver, sender, move |update| {
            updates.send(update).unwrap();
        })
        .run();

        let update = update_receiver.try_recv().unwrap();
        assert_eq!(
            update.result.unwrap(),
            Config {
                dictionary_keywords: vec!["meaning".into()],
                ..Config::default()
            }
        );
        let error = update.save_problem.unwrap();
        assert!(error.contains("conflicting setting"));
        assert!(!error.contains("meaning"));
        assert_eq!(fs::read_to_string(path).unwrap(), source);
    }

    #[test]
    fn a_later_edit_can_replace_an_uncommitted_save_without_a_false_conflict() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        let base = Config::default();
        fs::write(&path, toml::to_string(&base).unwrap()).unwrap();
        let (sender, receiver) = mpsc::channel();
        let (updates, received) = mpsc::channel();
        let mut worker = Worker::new(base_only(path.clone()), receiver, sender, move |update| {
            updates.send(update).unwrap();
        });
        worker.write = |_, _| Err(io::Error::other("blocked"));
        let first = Config {
            file_limit: 50,
            ..base.clone()
        };
        worker.process_saves(vec![Save {
            revision: 1,
            base,
            desired: first.clone(),
        }]);
        assert!(
            received
                .try_recv()
                .unwrap()
                .save_problem
                .unwrap()
                .contains("could not save")
        );
        worker.write = super::super::write;
        let latest = Config {
            file_limit: 30,
            ..first.clone()
        };
        worker.process_saves(vec![Save {
            revision: 2,
            base: first,
            desired: latest.clone(),
        }]);
        let saved = received.try_recv().unwrap();
        assert!(saved.save_problem.is_none());
        assert_eq!(saved.result.unwrap(), latest);
        assert_eq!(Config::parse(&fs::read_to_string(path).unwrap()).unwrap(), latest);
    }

    #[test]
    fn reverting_after_a_manual_repair_keeps_the_latest_edit() {
        for external_position in [dupin_core::Position::Center, dupin_core::Position::Top] {
            let temp = TempDir::new();
            let path = temp.path().join("config.toml");
            let base = Config {
                file_limit: 30,
                ..Config::default()
            };
            fs::write(&path, toml::to_string(&base).unwrap()).unwrap();
            let (sender, receiver) = mpsc::channel();
            let (updates, received) = mpsc::channel();
            let mut worker = Worker::new(base_only(path.clone()), receiver, sender, move |update| {
                updates.send(update).unwrap();
            });
            worker.write = |_, _| Err(io::Error::other("blocked"));
            let first = Config {
                file_limit: 50,
                ..base.clone()
            };
            worker.process_saves(vec![Save {
                revision: 1,
                base: base.clone(),
                desired: first.clone(),
            }]);
            assert!(received.try_recv().unwrap().save_problem.is_some());
            worker.write = super::super::write;
            let repaired = Config {
                position: external_position,
                ..first.clone()
            };
            fs::write(&path, toml::to_string(&repaired).unwrap()).unwrap();
            worker.emit_snapshot(1);
            assert_eq!(received.try_recv().unwrap().result.unwrap(), repaired);
            worker.process_saves(vec![Save {
                revision: 2,
                base: first,
                desired: base,
            }]);
            let saved = received.try_recv().unwrap();
            assert!(saved.save_problem.is_none());
            let expected = Config {
                file_limit: 30,
                ..repaired
            };
            assert_eq!(saved.result.unwrap(), expected);
            assert_eq!(
                Config::parse(&fs::read_to_string(path).unwrap()).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn reverting_an_uncommitted_edit_does_not_overwrite_a_third_value() {
        let original = Config {
            file_limit: 30,
            ..Config::default()
        };
        let pending = Config {
            file_limit: 50,
            ..original.clone()
        };
        let external = Config {
            file_limit: 40,
            ..original.clone()
        };
        assert!(
            merge(
                &pending,
                &original,
                &external,
                Some(&original),
                &LockedKeys::new()
            )
            .is_err()
        );
    }

    #[test]
    fn save_problems_survive_file_events_until_a_successful_save() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        let base = Config::default();
        let disk = Config {
            file_limit: 30,
            ..base.clone()
        };
        fs::write(&path, toml::to_string(&disk).unwrap()).unwrap();
        let (sender, receiver) = mpsc::channel();
        let (updates, received) = mpsc::channel();
        let mut worker = Worker::new(base_only(path.clone()), receiver, sender, move |update| {
            updates.send(update).unwrap();
        });
        worker.process_saves(vec![Save {
            revision: 1,
            base,
            desired: Config {
                file_limit: 50,
                ..Config::default()
            },
        }]);
        let failed = received.try_recv().unwrap();
        assert!(failed.save_problem.is_some());
        assert_eq!(failed.result.unwrap(), disk);
        fs::write(&path, format!("{}\n", toml::to_string(&disk).unwrap())).unwrap();
        worker.emit_snapshot(1);
        assert_eq!(received.try_recv().unwrap().save_problem, failed.save_problem);
        let desired = Config {
            position: dupin_core::Position::Top,
            ..disk.clone()
        };
        worker.process_saves(vec![Save {
            revision: 2,
            base: disk,
            desired: desired.clone(),
        }]);
        let saved = received.try_recv().unwrap();
        assert!(saved.save_problem.is_none());
        assert_eq!(saved.result.unwrap(), desired);
    }

    #[test]
    fn invalid_file_is_not_overwritten_by_a_settings_save() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        let source = "dictionary_keywords = [\"private-value\"\n";
        fs::write(&path, source).unwrap();
        let base = Config::default();
        let desired = Config {
            position: dupin_core::Position::Top,
            ..base.clone()
        };
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(base_only(path.clone()), move |update| {
            updates.send(update).unwrap()
        });

        let revision = service.save(&base, &desired).unwrap();
        let update = receive_revision(&receiver, revision);

        assert!(update.result.unwrap_err().contains("invalid"));
        assert_eq!(fs::read_to_string(path).unwrap(), source);
    }

    #[test]
    fn invalid_config_keeps_a_safe_location_and_recovers_on_a_later_change() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        let source = "hotkey = \"private-value\n";
        fs::write(&path, source).unwrap();
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(base_only(path.clone()), move |update| {
            updates.send(update).unwrap()
        });

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
        let service = Service::new(base_only(path.clone()), move |update| {
            updates.send(update).unwrap()
        });

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

        let first = roots([path.as_path()]);
        assert!(first.paths.contains(&canonical_root(&home)));
        assert!(
            first
                .paths
                .contains(&canonical_root(&generation_one.join("dupin")))
        );

        fs::remove_file(&config_dir).unwrap();
        symlink(&generation_two, &config_dir).unwrap();
        let second = roots([path.as_path()]);
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
        let linked_roots = roots([file_link.as_path()]);
        assert!(linked_roots.paths.contains(&canonical_root(temp.path())));
        assert!(linked_roots.paths.contains(&canonical_root(&external_one)));

        fs::remove_file(&file_link).unwrap();
        let next_target = external_two.join("config.toml");
        fs::write(&next_target, "").unwrap();
        symlink(&next_target, &file_link).unwrap();
        let repointed_roots = roots([file_link.as_path()]);
        assert!(repointed_roots.paths.contains(&canonical_root(&external_two)));
        assert!(!repointed_roots.paths.contains(&canonical_root(&external_one)));
    }

    #[test]
    fn roots_include_each_directory_symlink_hop() {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new();
        let home = temp.path().join("home");
        let links = temp.path().join("links");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for directory in [&home, &links, &first.join("dupin"), &second.join("dupin")] {
            fs::create_dir_all(directory).unwrap();
        }
        symlink(&first, links.join("current")).unwrap();
        symlink(links.join("current"), home.join(".config")).unwrap();
        let config = home.join(".config/dupin/config.toml");
        let initial = roots([config.as_path()]).paths;
        assert!(initial.contains(&canonical_root(&home)));
        assert!(initial.contains(&canonical_root(&links)));
        assert!(initial.contains(&canonical_root(&first.join("dupin"))));
        fs::remove_file(links.join("current")).unwrap();
        symlink(&second, links.join("current")).unwrap();
        let retargeted = roots([config.as_path()]).paths;
        assert!(retargeted.contains(&canonical_root(&links)));
        assert!(retargeted.contains(&canonical_root(&second.join("dupin"))));
        assert!(!retargeted.contains(&canonical_root(&first.join("dupin"))));
    }

    #[test]
    fn moving_the_config_directory_rebuilds_the_watch() {
        let temp = TempDir::new();
        let directory = temp.path().join("config");
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(&path, "file_limit = 37").unwrap();
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(base_only(path.clone()), move |update| {
            updates.send(update).unwrap()
        });
        service.activate();
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .result
                .unwrap()
                .file_limit,
            37
        );
        fs::rename(&directory, temp.path().join("moved")).unwrap();
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .result
                .unwrap(),
            Config::default()
        );
        fs::create_dir(&directory).unwrap();
        fs::write(&path, "file_limit = 58").unwrap();
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .result
                .unwrap()
                .file_limit,
            58
        );
        fs::write(&path, "file_limit = 59").unwrap();
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .result
                .unwrap()
                .file_limit,
            59
        );
    }

    #[test]
    fn real_watcher_reloads_changes_and_stops_with_the_service() {
        let temp = TempDir::new();
        let path = temp.path().join("config.toml");
        fs::write(&path, "position = \"center\"").unwrap();
        let (updates, receiver) = mpsc::channel();
        let service = Service::new(base_only(path.clone()), move |update| {
            updates.send(update).unwrap()
        });

        service.activate();
        let initial = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("settings watcher did not start");
        assert!(initial.watch_problem.is_none(), "{:?}", initial.watch_problem);
        assert_eq!(initial.result.unwrap().position, dupin_core::Position::Center);

        fs::write(&path, "position = \"top\"").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let changed = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let update = receiver
                .recv_timeout(remaining)
                .expect("settings watcher did not reload");
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
        stopped
            .recv_timeout(Duration::from_secs(5))
            .expect("settings watcher did not shut down");
    }
}
