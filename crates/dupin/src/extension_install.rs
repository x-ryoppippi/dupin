//! `dupin ext search|install|list|update|remove|sync`: `dupin-extension-install` pointed at
//! Dupin's directories and at the keywords the launcher already routes.

use std::path::{Path, PathBuf};

use dupin_broker::{Base, Broker, ConsentState, Local, PackageId};
use dupin_extension_install::{
    Change, Error, Found, Git, GitHub, Installer, LOCK_FILE, Listed, Outcomes, State, TOPIC,
};
use dupin_js_runtime::Diagnostic;
use dupin_storage::Storage;
use serde_json::{Value, json};

#[derive(Debug, PartialEq)]
pub enum Command {
    Install(String),
    List,
    Update(Option<String>),
    Remove(String),
    Sync,
}

/// `extensions.lock` in Dupin's data directory, beside settings.toml.
pub fn lock_path() -> Option<PathBuf> {
    crate::store::data_dir().map(|dir| dir.join(LOCK_FILE))
}

pub fn run(command: Command, json: bool) -> Result<Value, Diagnostic> {
    let bundled = crate::extensions::bundled::directory()
        .map(|directory| crate::extensions::bundled::packages(&directory))
        .unwrap_or_default();
    let installer = Installer {
        extensions: crate::extensions::directory()
            .ok_or_else(|| Diagnostic::new("no-directory", "neither XDG_CONFIG_HOME nor HOME is set"))?,
        lock: lock_path()
            .ok_or_else(|| Diagnostic::new("no-directory", "neither XDG_DATA_HOME nor HOME is set"))?,
        git: Git::default(),
        github: GitHub::default(),
        bundled: bundled.iter().map(|(_, name, _)| name.clone()).collect(),
    };
    let taken = routed_keywords(bundled);
    let cwd = std::env::current_dir().map_err(|error| Diagnostic::new("io-error", error.to_string()))?;
    let (lines, value) = match command {
        Command::Install(spec) => {
            let change = installer.install(&spec, &cwd, &taken).map_err(diagnostic)?;
            if change.previous.is_none() {
                forget_grants(&change.name)?;
                if let Err(failure) = delete_data(&change.name) {
                    installer.remove(&change.name).map_err(diagnostic)?;
                    return Err(Diagnostic::new(
                        "data-undeleted",
                        format!(
                            "Didn't install {}, because what an earlier package of that name \
                             stored must be deleted first. {}",
                            change.name, failure.message
                        ),
                    ));
                }
            }
            (describe(&change, "Installed"), change_json(&change))
        }
        Command::Update(name) => batch(
            installer.update(name.as_deref(), &taken).map_err(diagnostic)?,
            "Updated",
        )?,
        Command::Sync => batch(installer.sync(&taken).map_err(diagnostic)?, "Restored")?,
        Command::Remove(name) => {
            let entry = installer.remove(&name).map_err(diagnostic)?;
            forget_grants(&name)?;
            delete_data(&name)?;
            (
                vec![format!(
                    "Removed {name} {}, its grants and its stored data.",
                    entry.version
                )],
                json!({"removed": name}),
            )
        }
        Command::List => {
            let listed = installer.list().map_err(diagnostic)?;
            let mut broker = grants().0;
            let granted: Vec<_> = listed
                .iter()
                .map(|listed| granted(&mut broker, &installer.extensions.join(&listed.name)))
                .collect();
            let lines = if listed.is_empty() {
                vec!["No extensions were installed with `dupin ext install`.".to_owned()]
            } else {
                listed
                    .iter()
                    .zip(&granted)
                    .map(|(listed, granted)| list_line(listed, granted))
                    .collect()
            };
            let packages: Vec<_> = listed
                .iter()
                .zip(&granted)
                .map(|(listed, granted)| listed_json(listed, granted))
                .collect();
            (lines, json!({ "packages": packages }))
        }
    };
    if json {
        return Ok(value);
    }
    for line in lines {
        crate::out::outln!("{line}");
    }
    Ok(Value::Null)
}

/// The broker over the grant files: the declarative base Home Manager writes, and the grants
/// chosen in Dupin with the path they are saved to. A local file that exists but can't be read
/// grants nothing and is never written.
fn grants() -> (Broker, Option<std::path::PathBuf>) {
    let read =
        |path: &Option<std::path::PathBuf>| path.as_ref().and_then(|path| std::fs::read_to_string(path).ok());
    let base = crate::extensions::access::base_path()
        .map(|path| Base::read(&path))
        .unwrap_or_default();
    let path = crate::extensions::access::local_path();
    let local = match (read(&path), path.as_ref().is_some_and(|path| path.exists())) {
        (Some(source), _) => Local::parse(&source).unwrap_or_else(|_| Local::unreadable()),
        (None, true) => Local::unreadable(),
        (None, false) => Local::default(),
    };
    (Broker::new(base, local), path)
}

/// The scopes of the package in `folder` granted for the revision installed there.
fn granted(broker: &mut Broker, folder: &std::path::Path) -> Vec<String> {
    let Ok((id, revision, capabilities)) = crate::extensions::declaration_of(folder) else {
        return Vec::new();
    };
    let _ = broker.install(id.clone(), revision, capabilities);
    broker
        .consents(&id)
        .into_iter()
        .filter(|(_, state)| matches!(state, ConsentState::Granted(_)))
        .map(|(scope, _)| scope.key().to_string())
        .collect()
}

/// Uninstalling forgets the user's decisions, and so does installing a name afresh, since the
/// decisions left under it were about another package. A running Dupin reads the file again
/// before it next decides or saves.
fn forget_grants(name: &str) -> Result<(), Diagnostic> {
    match crate::extensions::access::local_path() {
        Some(path) => forget_grants_in(&path, name),
        None => Ok(()),
    }
}

/// [`forget_grants`] over the local grant file at `path`.
fn forget_grants_in(path: &Path, name: &str) -> Result<(), Diagnostic> {
    let Ok(id) = PackageId::new(name) else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    let unsaved = |message: String| {
        Diagnostic::new("grants-unsaved", message).file(crate::extensions::access::LOCAL_FILE)
    };
    let source = std::fs::read_to_string(path).map_err(|error| unsaved(error.to_string()))?;
    let local = Local::parse(&source).map_err(|error| unsaved(error.to_string()))?;
    let mut broker = Broker::new(Base::default(), local);
    broker.uninstall(&id);
    broker.save(path).map_err(|error| unsaved(error.to_string()))
}

/// `dupin ext eject NAME`: copies the bundled package into the extensions folder. The copy
/// starts afresh, as a name installed afresh does: a changed copy ejected before and deleted
/// left its decisions and stored data under the same name, and they were about other code.
pub fn eject(name: &str) -> Result<PathBuf, Diagnostic> {
    let bundled = crate::extensions::bundled::directory()
        .ok_or_else(|| Diagnostic::new("no-bundled", "this build of Dupin has no bundled extensions"))?;
    let user = crate::extensions::directory().ok_or_else(|| {
        Diagnostic::new(
            "no-directory",
            "couldn't find the extensions folder beside config.toml",
        )
    })?;
    eject_into(
        &bundled,
        &user,
        crate::extensions::access::local_path().as_deref(),
        crate::extensions::access::storage().as_ref(),
        name,
    )
}

/// [`eject`] from the bundled packages in `bundled` to the extensions folder `user`,
/// forgetting `name`'s decisions in the grant file `grants` and its data in `storage`. A copy
/// whose earlier decisions or data can't be forgotten is removed again, so it never runs with
/// them.
pub(crate) fn eject_into(
    bundled: &Path,
    user: &Path,
    grants: Option<&Path>,
    storage: Option<&Storage>,
    name: &str,
) -> Result<PathBuf, Diagnostic> {
    let copy = crate::extensions::bundled::eject(bundled, user, name)?;
    let forgotten = grants
        .map_or(Ok(()), |grants| forget_grants_in(grants, name))
        .and_then(|()| storage.map_or(Ok(()), |storage| delete_data_in(storage, name)));
    if let Err(failure) = forgotten {
        let _ = std::fs::remove_dir_all(&copy);
        return Err(Diagnostic::new(
            failure.code.clone(),
            format!(
                "Didn't eject {name}, because what an earlier copy of it was allowed or stored \
                 must be forgotten first. {}",
                failure.message
            ),
        ));
    }
    Ok(copy)
}

/// Uninstalling deletes what the package stored, and so does installing a name afresh, since a
/// package's namespace is named after its package name; revoking its `storage` in Settings
/// keeps it. A running Dupin that has the package's data open deletes it itself.
fn delete_data(name: &str) -> Result<(), Diagnostic> {
    match crate::extensions::access::storage() {
        Some(storage) => delete_data_in(&storage, name),
        None => Ok(()),
    }
}

/// [`delete_data`] from `storage`.
fn delete_data_in(storage: &Storage, name: &str) -> Result<(), Diagnostic> {
    let Ok(id) = PackageId::new(name) else {
        return Ok(());
    };
    let namespace = dupin_extension_bridge::storage::namespace(&id, dupin_extension_bridge::Origin::User);
    let unsaved = |message: String| Diagnostic::new("data-undeleted", message);
    match storage.discard(&namespace) {
        Ok(()) => Ok(()),
        Err(dupin_storage::Error::Busy) => {
            crate::control::exchange(crate::control::Command::ExtensionDataDelete {
                package: name.to_owned(),
            })
            .map(|_| ())
            .map_err(|error| unsaved(format!("Dupin couldn't delete {name}'s stored data: {error}")))
        }
        Err(error) => Err(unsaved(format!("Couldn't delete {name}'s stored data: {error}"))),
    }
}

/// Who already answers a keyword in the launcher: a bundled extension, or a command of the
/// effective configuration.
fn routed_keywords(bundled: Vec<(PathBuf, String, String)>) -> impl Fn(&str) -> Option<String> {
    let (config, ..) = crate::settings::load(&crate::settings::Paths::discover());
    let commands = crate::commands::configured(&config).ok();
    move |keyword| {
        if let Some((_, name, _)) = bundled.iter().find(|(.., bundled)| bundled == keyword) {
            return Some(format!("the bundled `{name}` extension"));
        }
        let commands = commands.as_ref()?;
        if commands.package_aliases.iter().any(|(alias, _)| alias == keyword) {
            return Some("an alias in config.toml".to_owned());
        }
        Some(match commands.registry.holder(keyword)? {
            dupin_core::KeywordHolder::Command(command) => format!("the `{command}` command"),
            dupin_core::KeywordHolder::Web(_) => "a web search in config.toml".to_owned(),
            dupin_core::KeywordHolder::Path(_) => "an alias in config.toml".to_owned(),
        })
    }
}

fn batch(results: Outcomes, verb: &str) -> Result<(Vec<String>, Value), Diagnostic> {
    if results.is_empty() {
        return Ok((vec!["Nothing to do.".into()], json!({"packages": []})));
    }
    let mut lines = Vec::new();
    let mut values = Vec::new();
    let mut failed = None;
    for (name, result) in results {
        match result {
            Ok(change) => {
                lines.extend(describe(&change, verb));
                values.push(change_json(&change));
            }
            Err(error) => {
                lines.push(format!("{name}: {error}"));
                values.push(json!({"name": name, "error": error.to_string(), "code": error.code}));
                failed.get_or_insert((name, error));
            }
        }
    }
    if let Some((name, error)) = failed {
        for line in &lines {
            eprintln!("{line}");
        }
        return Err(diagnostic(Error {
            message: format!("{name}: {}", error.message),
            ..error
        }));
    }
    Ok((lines, json!({"packages": values})))
}

fn describe(change: &Change, verb: &str) -> Vec<String> {
    let entry = &change.entry;
    if change.unchanged() && verb != "Restored" {
        return vec![format!("{} {} is up to date.", change.name, entry.version)];
    }
    let at = match (&entry.rev, &entry.commit) {
        (Some(rev), Some(commit)) => format!(" at {rev} ({})", short(commit)),
        (None, Some(commit)) => format!(" at {}", short(commit)),
        _ => String::new(),
    };
    let mut lines = vec![format!(
        "{verb} {} {} ({}, keyword `{}`) from {}{at}.",
        change.name, entry.version, change.title, entry.keyword, entry.source
    )];
    if change.previous.is_none() && !entry.requested.is_empty() {
        lines.push(format!(
            "  Requests {}; you choose which to grant when you turn it on.",
            entry.requested.join(", ")
        ));
    }
    if !change.added.is_empty() {
        lines.push(format!(
            "  Adds {}; it can't use them until you agree.",
            change.added.join(", ")
        ));
    }
    if !change.removed.is_empty() {
        lines.push(format!(
            "  Drops {}; their grants are removed.",
            change.removed.join(", ")
        ));
    }
    lines
}

fn list_line(listed: &Listed, granted: &[String]) -> String {
    let entry = &listed.entry;
    let pinned = entry
        .commit
        .as_deref()
        .map(|commit| format!(" @ {}", short(commit)))
        .unwrap_or_default();
    let state = match &listed.state {
        State::Installed => String::new(),
        State::Missing => " — missing; run `dupin ext sync`".into(),
        State::Modified(reason) => format!(" — disabled: {reason}; run `dupin ext sync`"),
    };
    let granted = if granted.is_empty() {
        String::new()
    } else {
        format!(" — granted {}", granted.join(", "))
    };
    format!(
        "{} {} (`{}`) from {}{pinned}{state}{granted}",
        listed.name, entry.version, entry.keyword, entry.source
    )
}

fn change_json(change: &Change) -> Value {
    json!({
        "name": change.name,
        "version": change.entry.version,
        "keyword": change.entry.keyword,
        "source": change.entry.source,
        "ref": change.entry.rev,
        "commit": change.entry.commit,
        "content_hash": change.entry.content_hash,
        "changed": !change.unchanged(),
        "requested": change.entry.requested,
        "added": change.added,
        "removed": change.removed,
    })
}

fn listed_json(listed: &Listed, granted: &[String]) -> Value {
    let (state, problem) = match &listed.state {
        State::Installed => ("installed", None),
        State::Missing => ("missing", None),
        State::Modified(reason) => ("modified", Some(reason)),
    };
    json!({
        "name": listed.name,
        "state": state,
        "problem": problem,
        "version": listed.entry.version,
        "keyword": listed.entry.keyword,
        "source": listed.entry.source,
        "ref": listed.entry.rev,
        "commit": listed.entry.commit,
        "content_hash": listed.entry.content_hash,
        "installed_at": listed.entry.installed_at,
        "requested": listed.entry.requested,
        "granted": granted,
    })
}

fn diagnostic(error: Error) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(error.code, error.message);
    if let Some(file) = error.file {
        diagnostic = diagnostic.file(file);
    }
    if let Some(field) = error.field {
        diagnostic = diagnostic.field(field);
    }
    if let Some(hint) = error.hint {
        diagnostic = diagnostic.hint(hint);
    }
    diagnostic
}

fn short(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

/// What `dupin ext search` says before any listing: nobody has looked at these packages.
const UNREVIEWED: &str = "Unreviewed: anyone can tag a GitHub repository `dupin-extension`, and nobody \
                          checks these before they are listed. Read a package before you install it; \
                          nothing it requests is granted until you agree.";

/// How many scopes a listing names before it counts the rest.
const SHOWN_SCOPES: usize = 8;

/// `dupin ext search [QUERY]`: lists, and never installs, the repositories tagged
/// `dupin-extension` that match `query`.
pub fn search(query: Option<&str>, json: bool) -> Result<Value, Diagnostic> {
    let found = GitHub::default().search(query).map_err(diagnostic)?;
    if json {
        return Ok(found_json(&found));
    }
    for line in found_lines(&found) {
        crate::out::outln!("{line}");
    }
    Ok(Value::Null)
}

fn found_lines(found: &Found) -> Vec<String> {
    let mut lines = vec![UNREVIEWED.to_owned()];
    if found.repositories.is_empty() {
        lines.push(String::new());
        lines.push(format!("No repositories tagged `{TOPIC}` match."));
    }
    for repository in &found.repositories {
        lines.push(String::new());
        let pushed = repository
            .pushed
            .as_deref()
            .map(|day| format!(", last push {day}"))
            .unwrap_or_default();
        lines.push(format!(
            "{}/{} ({} stars{pushed})",
            repository.owner, repository.repo, repository.stars
        ));
        if let Some(description) = &repository.description {
            lines.push(format!("  {description}"));
        }
        lines.push(match &repository.package {
            Ok(package) => {
                let scopes = &package.capabilities;
                let requests = if scopes.is_empty() {
                    "requests nothing".to_owned()
                } else {
                    let mut named = scopes[..scopes.len().min(SHOWN_SCOPES)].join(", ");
                    if scopes.len() > SHOWN_SCOPES {
                        named.push_str(&format!(" and {} more", scopes.len() - SHOWN_SCOPES));
                    }
                    format!("requests {named}")
                };
                format!(
                    "  {} {} (`{}`, keyword `{}`); on `{}` it {requests}",
                    package.title, package.version, package.name, package.keyword, repository.default_branch
                )
            }
            Err(problem) => format!("  Can't be read as a package: {problem}"),
        });
        lines.push(format!("  {}", repository.install_command()));
    }
    let listed = found.repositories.len() as u64;
    if found.total > listed {
        lines.push(String::new());
        lines.push(format!(
            "GitHub found {} repositories; these are the {listed} with the most stars. Add words to narrow the search.",
            found.total
        ));
    }
    if found.incomplete {
        lines.push("GitHub stopped searching early, so some repositories may be missing.".to_owned());
    }
    lines
}

fn found_json(found: &Found) -> Value {
    let repositories: Vec<_> = found
        .repositories
        .iter()
        .map(|repository| {
            let (package, problem) = match &repository.package {
                Ok(package) => (
                    json!({
                        "name": package.name,
                        "version": package.version,
                        "title": package.title,
                        "keyword": package.keyword,
                        "requested": package.capabilities,
                    }),
                    None,
                ),
                Err(problem) => (Value::Null, Some(problem)),
            };
            json!({
                "owner": repository.owner,
                "repo": repository.repo,
                "source": repository.source(),
                "install": repository.install_command(),
                "description": repository.description,
                "stars": repository.stars,
                "pushed": repository.pushed,
                "default_branch": repository.default_branch,
                "package": package,
                "problem": problem,
            })
        })
        .collect();
    json!({
        "reviewed": false,
        "topic": TOPIC,
        "total": found.total,
        "incomplete": found.incomplete,
        "repositories": repositories,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_extension_install::{Manifest, Repository};

    fn repository(package: Result<Manifest, String>) -> Repository {
        Repository {
            owner: "someone".into(),
            repo: "dupin-word-count".into(),
            description: Some("Counts words".into()),
            stars: 42,
            pushed: Some("2026-10-01".into()),
            default_branch: "main".into(),
            package,
        }
    }

    #[test]
    fn search_results_say_they_are_unreviewed_and_how_to_install_each() {
        let found = Found {
            total: 31,
            incomplete: false,
            repositories: vec![
                repository(Ok(Manifest {
                    name: "word-count".into(),
                    version: "1.2.0".into(),
                    title: "Word Count".into(),
                    keyword: "wc".into(),
                    capabilities: (0..10).map(|n| format!("files:notes-{n}")).collect(),
                })),
                repository(Err("no package.json on `main`".into())),
            ],
        };
        let lines = found_lines(&found);
        assert!(lines[0].starts_with("Unreviewed:"), "{lines:#?}");
        assert_eq!(
            lines[2],
            "someone/dupin-word-count (42 stars, last push 2026-10-01)"
        );
        assert!(lines[4].starts_with(
            "  Word Count 1.2.0 (`word-count`, keyword `wc`); on `main` it requests files:notes-0,"
        ));
        assert!(lines[4].ends_with("files:notes-7 and 2 more"), "{}", lines[4]);
        assert_eq!(lines[5], "  dupin ext install github:someone/dupin-word-count");
        assert_eq!(
            lines[9],
            "  Can't be read as a package: no package.json on `main`"
        );
        assert!(lines.last().unwrap().starts_with("GitHub found 31 repositories"));

        let value = found_json(&found);
        assert_eq!(value["reviewed"], false);
        assert_eq!(
            value["repositories"][0]["install"],
            "dupin ext install github:someone/dupin-word-count"
        );
        assert_eq!(
            value["repositories"][0]["package"]["requested"]
                .as_array()
                .unwrap()
                .len(),
            10
        );
        assert_eq!(value["repositories"][1]["problem"], "no package.json on `main`");
    }
}
