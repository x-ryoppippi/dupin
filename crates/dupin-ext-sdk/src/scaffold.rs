use std::path::{Path, PathBuf};

use dupin_js_runtime::Diagnostic;
use serde_json::Value;

use crate::DECLARATIONS;
use crate::manifest::MANIFEST_FILE;

const TS_SOURCE: &str = "src/main.ts";
const TS_MAIN: &str = "dist/main.js";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    /// `main.js`, run as written.
    JavaScript,
    /// `src/main.ts`, bundled into `dist/main.js`, with a workflow that commits the bundle.
    TypeScript,
}

/// Creates a package in `dir`, which must not exist or be empty. Its name is the folder's name.
/// Returns the files written, relative to `dir`.
pub fn scaffold(dir: &Path, template: Template) -> Result<Vec<PathBuf>, Diagnostic> {
    let name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(Diagnostic::new(
            "invalid-name",
            "The folder's name becomes the extension's id, so it must be 1–64 lowercase letters, digits or hyphens",
        )
        .hint("Use a folder name such as `word-count`."));
    }
    if std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some()) {
        return Err(Diagnostic::new("folder-not-empty", "already has files in it")
            .file(dir.display().to_string())
            .hint("Pick a new folder; `dupin ext new` never overwrites files."));
    }

    let title = title_case(&name);
    let files = files(&name, &title, template);
    for (path, contents) in &files {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| unwritable(&path, &error))?;
        }
        std::fs::write(&path, contents).map_err(|error| unwritable(&path, &error))?;
    }
    Ok(files.into_iter().map(|(path, _)| PathBuf::from(path)).collect())
}

fn files(name: &str, title: &str, template: Template) -> Vec<(&'static str, String)> {
    // Written by hand to keep the conventional key order; serde_json's maps sort their keys.
    let string = |value: &str| Value::from(value).to_string();
    let (source, main) = match template {
        Template::JavaScript => (String::new(), String::new()),
        Template::TypeScript => (
            format!("\n\t\"source\": {},", string(TS_SOURCE)),
            format!(",\n\t\t\"main\": {}", string(TS_MAIN)),
        ),
    };
    let manifest = format!(
        "{{\n\t\"name\": {name},\n\t\"version\": \"0.1.0\",\n\t\"private\": true,\n\t\"type\": \"module\",\n\t\"keywords\": [\"dupin-extension\"],{source}\n\t\"dupin\": {{\n\t\t\"title\": {title},\n\t\t\"keyword\": {name}{main}\n\t}}\n}}\n",
        name = string(name),
        title = string(title),
    );

    let (build_note, build_section) = match template {
        Template::JavaScript => ("", String::new()),
        Template::TypeScript => (
            "bundles `src/main.ts` with Bun or esbuild, ",
            format!(
                "\n## Build\n\n```sh\ndupin ext build .\n```\n\n`build` bundles `{TS_SOURCE}` and its npm dependencies into one module at `{TS_MAIN}`, and fails if a dependency needs a Node built-in. Commit `{TS_MAIN}`: Dupin runs it as committed. `.github/workflows/build.yml` rebuilds and commits it on every push to `main`.\n"
            ),
        ),
    };
    let fill = |text: &str| {
        text.replace("{{title}}", title)
            .replace("{{keyword}}", name)
            .replace("{{source}}", TS_SOURCE)
            .replace("{{main}}", TS_MAIN)
            .replace("{{build_note}}", build_note)
            .replace("{{build_section}}", &build_section)
    };

    let mut files = vec![
        (MANIFEST_FILE, manifest),
        ("dupin.d.ts", DECLARATIONS.to_owned()),
        (
            "fixtures/search.json",
            include_str!("../templates/fixture.json").to_owned(),
        ),
        ("README.md", fill(include_str!("../templates/README.md"))),
    ];
    match template {
        Template::JavaScript => files.push(("main.js", include_str!("../templates/main.js").to_owned())),
        Template::TypeScript => files.extend([
            (TS_SOURCE, include_str!("../templates/main.ts").to_owned()),
            (
                "tsconfig.json",
                include_str!("../templates/tsconfig.json").to_owned(),
            ),
            (".gitignore", include_str!("../templates/gitignore").to_owned()),
            (
                ".github/workflows/build.yml",
                fill(include_str!("../templates/build.yml")),
            ),
        ]),
    }
    files
}

fn title_case(name: &str) -> String {
    name.split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn unwritable(path: &Path, error: &std::io::Error) -> Diagnostic {
    Diagnostic::new("unwritable", format!("Couldn't write it: {error}")).file(path.display().to_string())
}
