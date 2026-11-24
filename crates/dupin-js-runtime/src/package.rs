use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::Diagnostic;

const MAX_SOURCE_FILES: usize = 256;
/// Every JavaScript file of a package together, as read from disk.
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_FOLDER_DEPTH: usize = 8;

/// The JavaScript files of the package in `root`, as names relative to it (`lib/util.js`)
/// and their canonical paths. A file or folder that resolves outside `root` is an error, so
/// a symlink cannot bring outside code into the package.
pub fn source_files(root: &Path) -> Result<Vec<(String, PathBuf)>, Diagnostic> {
    let canonical_root = root.canonicalize().map_err(|error| {
        Diagnostic::new("package-unreadable", format!("Couldn't read the folder: {error}"))
    })?;
    let mut files = Vec::new();
    walk(&canonical_root, &canonical_root, "", 0, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(
    root: &Path,
    folder: &Path,
    prefix: &str,
    depth: usize,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), Diagnostic> {
    let entries = std::fs::read_dir(folder).map_err(|error| {
        Diagnostic::new("package-unreadable", format!("Couldn't read the folder: {error}"))
            .file(display_prefix(prefix))
    })?;
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let relative = format!("{prefix}{name}");
        let path = entry.path();
        let is_source = is_source_name(&name);
        let followed = path.metadata();
        let is_dir = followed.as_ref().is_ok_and(|metadata| metadata.is_dir());
        if !is_dir && !is_source {
            continue;
        }
        let canonical = path.canonicalize().map_err(|error| {
            Diagnostic::new("package-unreadable", format!("Couldn't read it: {error}")).file(&relative)
        })?;
        if !canonical.starts_with(root) {
            return Err(Diagnostic::new(
                "symlink-escape",
                "is a symlink to something outside the package folder",
            )
            .file(&relative)
            .hint("Copy the file into the package instead of linking to it."));
        }
        if is_dir {
            if folder.starts_with(&canonical) {
                return Err(
                    Diagnostic::new("symlink-loop", "links back to a folder that contains it")
                        .file(&relative)
                        .hint("Remove the link."),
                );
            }
            if depth + 1 >= MAX_FOLDER_DEPTH {
                return Err(Diagnostic::new(
                    "package-too-deep",
                    format!("nests folders more than {MAX_FOLDER_DEPTH} deep"),
                )
                .file(&relative)
                .hint("Bundle the extension into one module with `dupin ext build`."));
            }
            walk(root, &canonical, &format!("{relative}/"), depth + 1, files)?;
        } else {
            files.push((relative, canonical));
            if files.len() > MAX_SOURCE_FILES {
                return Err(Diagnostic::new(
                    "too-many-files",
                    format!("The package has more than {MAX_SOURCE_FILES} JavaScript files"),
                )
                .hint("Bundle the extension into one module with `dupin ext build`."));
            }
        }
    }
    Ok(())
}

fn display_prefix(prefix: &str) -> String {
    match prefix.trim_end_matches('/') {
        "" => ".".into(),
        folder => folder.into(),
    }
}

fn is_source_name(name: &str) -> bool {
    name.ends_with(".js") || name.ends_with(".mjs")
}

/// Reads every source file of the package, within [`MAX_SOURCE_BYTES`] in total.
pub(crate) fn read_sources(files: &[(String, PathBuf)]) -> Result<Vec<(String, String)>, Diagnostic> {
    let mut remaining = MAX_SOURCE_BYTES;
    let mut sources = Vec::with_capacity(files.len());
    for (name, path) in files {
        let too_large = || {
            Diagnostic::new(
                "package-too-large",
                format!(
                    "The package's JavaScript is larger than {} MiB",
                    MAX_SOURCE_BYTES / 1024 / 1024
                ),
            )
            .file(name)
            .hint("Bundle and minify the extension, and leave out code it doesn't run.")
        };
        let file = std::fs::File::open(path).map_err(|error| {
            Diagnostic::new("package-unreadable", format!("Couldn't read it: {error}")).file(name)
        })?;
        let mut bytes = Vec::new();
        file.take(remaining as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                Diagnostic::new("package-unreadable", format!("Couldn't read it: {error}")).file(name)
            })?;
        if bytes.len() > remaining {
            return Err(too_large());
        }
        remaining -= bytes.len();
        let source = String::from_utf8(bytes).map_err(|_| {
            Diagnostic::new("not-utf8", "is not UTF-8 text")
                .file(name)
                .hint("Save the file as UTF-8.")
        })?;
        sources.push((name.clone(), source));
    }
    Ok(sources)
}

/// Resolves `specifier`, imported by the module `base`, to a module name: the host module, or a
/// package file found by a relative path. Paths are resolved by name alone, never on disk, so
/// only files [`source_files`] accepted can load.
pub(crate) fn resolve(
    exists: &dyn Fn(&str) -> bool,
    base: &str,
    specifier: &str,
) -> Result<String, Diagnostic> {
    if specifier == crate::HOST_MODULE {
        return Ok(specifier.into());
    }
    // The host itself imports the entry by its package name.
    if base.is_empty() && exists(specifier) {
        return Ok(specifier.into());
    }
    let located = |diagnostic: Diagnostic| diagnostic.file(base).field(format!("import \"{specifier}\""));
    if !(specifier.starts_with("./") || specifier.starts_with("../")) {
        return Err(located(Diagnostic::new(
            "import-unavailable",
            "is neither a relative path nor `dupin`",
        ))
        .hint(
            "Import package files with `./`, and bundle dependencies with `dupin ext build` before installing.",
        ));
    }
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for component in Path::new(specifier).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(located(Diagnostic::new(
                        "import-escape",
                        "leads outside the package folder",
                    ))
                    .hint("Keep every file the extension imports inside its folder."));
                }
            }
            Component::Normal(part) => {
                let Some(part) = part.to_str() else {
                    return Err(located(Diagnostic::new("import-missing", "is not valid UTF-8")));
                };
                parts.push(part);
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(located(Diagnostic::new("import-escape", "is an absolute path"))
                    .hint("Import package files with a relative path such as `./util.js`."));
            }
        }
    }
    let name = parts.join("/");
    if exists(&name) {
        Ok(name)
    } else {
        let hint = if is_source_name(&name) {
            "Check the path; imports are relative to the importing file."
        } else {
            "Write the file's extension in the import, such as `./util.js`."
        };
        Err(located(Diagnostic::new(
            "import-missing",
            format!("names `{name}`, which is not a JavaScript file in the package"),
        ))
        .hint(hint))
    }
}
