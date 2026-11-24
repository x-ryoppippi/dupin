//! `dupin ext install|list|update|remove|sync`: `dupin-extension-install` pointed at Dupin's
//! directories and at the keywords the launcher already routes.

use std::path::PathBuf;

use dupin_extension_install::{Change, Error, Git, Installer, LOCK_FILE, Listed, Outcomes, State, grants};
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
    let installer = Installer {
        extensions: crate::extensions::directory()
            .ok_or_else(|| Diagnostic::new("no-directory", "neither XDG_CONFIG_HOME nor HOME is set"))?,
        lock: lock_path()
            .ok_or_else(|| Diagnostic::new("no-directory", "neither XDG_DATA_HOME nor HOME is set"))?,
        git: Git::default(),
    };
    let taken = routed_keywords();
    let cwd = std::env::current_dir().map_err(|error| Diagnostic::new("io-error", error.to_string()))?;
    let (lines, value) = match command {
        Command::Install(spec) => {
            let change = installer.install(&spec, &cwd, &taken).map_err(diagnostic)?;
            (describe(&change, "Installed"), change_json(&change))
        }
        Command::Update(name) => batch(
            installer.update(name.as_deref(), &taken).map_err(diagnostic)?,
            "Updated",
        )?,
        Command::Sync => batch(installer.sync(&taken).map_err(diagnostic)?, "Restored")?,
        Command::Remove(name) => {
            let entry = installer.remove(&name).map_err(diagnostic)?;
            (
                vec![format!("Removed {name} {} and its grants.", entry.version)],
                json!({"removed": name}),
            )
        }
        Command::List => {
            let mut listed = installer.list().map_err(diagnostic)?;
            let base = grants_base()?;
            for listed in &mut listed {
                listed.entry.granted =
                    grants::effective(&base, Some(&listed.entry), &listed.name, &listed.entry.requested)
                        .into_iter()
                        .collect();
            }
            let lines = if listed.is_empty() {
                vec!["No extensions were installed with `dupin ext install`.".to_owned()]
            } else {
                listed.iter().map(list_line).collect()
            };
            (
                lines,
                json!({"packages": listed.iter().map(listed_json).collect::<Vec<_>>()}),
            )
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

/// The grants Home Manager declares in extension-grants.toml beside config.toml, which local
/// grants overlay.
fn grants_base() -> Result<grants::Base, Diagnostic> {
    match crate::settings::path()
        .as_deref()
        .and_then(std::path::Path::parent)
    {
        Some(dir) => grants::Base::read(&dir.join(grants::GRANTS_FILE)).map_err(diagnostic),
        None => Ok(grants::Base::default()),
    }
}

/// Who already answers a keyword in the launcher, from the effective configuration.
fn routed_keywords() -> impl Fn(&str) -> Option<String> {
    let (config, ..) = crate::settings::load(&crate::settings::Paths::discover());
    let commands = crate::commands::configured(&config).ok();
    move |keyword| {
        let commands = commands.as_ref()?;
        if let Some(command) = commands.registry.keyword(keyword) {
            return Some(format!("the `{command}` command"));
        }
        commands
            .web
            .has_keyword(keyword)
            .then(|| "a web search in config.toml".to_owned())
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

fn list_line(listed: &Listed) -> String {
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
    let pending = if entry.pending_consent.is_empty() {
        String::new()
    } else {
        format!(" — awaiting consent for {}", entry.pending_consent.join(", "))
    };
    let granted = if entry.granted.is_empty() {
        String::new()
    } else {
        format!(" — granted {}", entry.granted.join(", "))
    };
    format!(
        "{} {} (`{}`) from {}{pinned}{state}{granted}{pending}",
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
        "pending_consent": change.entry.pending_consent,
    })
}

fn listed_json(listed: &Listed) -> Value {
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
        "granted": listed.entry.granted,
        "pending_consent": listed.entry.pending_consent,
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
