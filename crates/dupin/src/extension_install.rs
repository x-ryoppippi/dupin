//! `dupin ext install|list|update|remove|sync`: `dupin-extension-install` pointed at Dupin's
//! directories and at the keywords the launcher already routes.

use std::path::PathBuf;

use dupin_broker::{Base, Broker, ConsentState, Local, PackageId};
use dupin_extension_install::{Change, Error, Git, GitHub, Installer, LOCK_FILE, Listed, Outcomes, State};
use dupin_js_runtime::Diagnostic;
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
        println!("{line}");
    }
    Ok(Value::Null)
}

/// The broker over the grant files: the declarative base Home Manager writes, and the grants
/// chosen in Dupin with the path they are saved to. A local file that exists but can't be read
/// grants nothing and is never written.
fn grants() -> (Broker, Option<std::path::PathBuf>) {
    let read =
        |path: &Option<std::path::PathBuf>| path.as_ref().and_then(|path| std::fs::read_to_string(path).ok());
    let base = read(&crate::extensions::access::base_path())
        .and_then(|source| Base::parse(&source).ok())
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
    let (mut broker, Some(path)) = grants() else {
        return Ok(());
    };
    let Ok(id) = PackageId::new(name) else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    broker.uninstall(&id);
    broker.save(&path).map_err(|error| {
        Diagnostic::new("grants-unsaved", error.to_string()).file(crate::extensions::access::LOCAL_FILE)
    })
}

/// Uninstalling deletes what the package stored, and so does installing a name afresh, since a
/// package's namespace is named after its package name; revoking its `storage` in Settings
/// keeps it. A running Dupin that has the package's data open deletes it itself.
fn delete_data(name: &str) -> Result<(), Diagnostic> {
    let Ok(id) = PackageId::new(name) else {
        return Ok(());
    };
    let Some(storage) = crate::extensions::access::storage() else {
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
        let command = commands.as_ref()?.registry.keyword(keyword)?;
        Some(format!("the `{command}` command"))
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
