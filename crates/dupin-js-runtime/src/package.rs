use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::Diagnostic;
use crate::typescript::is_typescript;

const MAX_SOURCE_FILES: usize = 256;
/// Every module of a package together, as read from disk.
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_FOLDER_DEPTH: usize = 8;

/// The modules of the package in `root` whose entry is `entry`, as names relative to it
/// (`lib/util.js`) and their canonical paths: its JavaScript files and, when the entry is
/// TypeScript, its TypeScript files. A package with a JavaScript entry may keep the TypeScript
/// its bundle was built from beside it. A file or folder that resolves outside `root` is an
/// error, so a symlink cannot bring outside code into the package.
pub fn source_files(root: &Path, entry: &str) -> Result<Vec<(String, PathBuf)>, Diagnostic> {
    let canonical_root = root.canonicalize().map_err(|error| {
        Diagnostic::new("package-unreadable", format!("Couldn't read the folder: {error}"))
    })?;
    let mut files = Vec::new();
    let typescript = is_typescript(entry);
    walk(&canonical_root, &canonical_root, "", 0, typescript, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(
    root: &Path,
    folder: &Path,
    prefix: &str,
    depth: usize,
    typescript: bool,
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
        let is_source = is_javascript(&name) || (typescript && is_typescript(&name));
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
                .hint("Bundle the extension into fewer files with esbuild or Bun."));
            }
            walk(
                root,
                &canonical,
                &format!("{relative}/"),
                depth + 1,
                typescript,
                files,
            )?;
        } else {
            files.push((relative, canonical));
            if files.len() > MAX_SOURCE_FILES {
                return Err(too_many_files());
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

fn is_javascript(name: &str) -> bool {
    name.ends_with(".js") || name.ends_with(".mjs")
}

/// Whether the file `name` can be a package's entry: JavaScript, or TypeScript other than a
/// declaration file.
pub fn is_module(name: &str) -> bool {
    is_javascript(name) || is_typescript(name)
}

/// A package's modules, read from its folder within the package limits. Dupin reads it and
/// hands it to the extension host, which alone parses and compiles it.
#[derive(Clone, Debug)]
pub struct Sources {
    entry: String,
    files: Vec<(String, String)>,
}

impl Sources {
    /// Reads every module [`source_files`] finds in `root`. `entry` is the module that exports
    /// `search`, relative to `root`.
    pub fn read(root: &Path, entry: &str) -> Result<Self, Diagnostic> {
        let entry = entry.strip_prefix("./").unwrap_or(entry);
        let files = source_files(root, entry)?;
        if !files.iter().any(|(name, _)| name == entry) {
            return Err(missing_entry(entry));
        }
        let mut remaining = MAX_SOURCE_BYTES;
        let mut sources = Vec::with_capacity(files.len());
        for (name, path) in &files {
            let unreadable = |error: std::io::Error| {
                Diagnostic::new("package-unreadable", format!("Couldn't read it: {error}")).file(name)
            };
            let file = std::fs::File::open(path).map_err(unreadable)?;
            let mut bytes = Vec::new();
            file.take(remaining as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(unreadable)?;
            if bytes.len() > remaining {
                return Err(too_large(name));
            }
            remaining -= bytes.len();
            let source = String::from_utf8(bytes).map_err(|_| not_utf8(name))?;
            sources.push((name.clone(), source));
        }
        Ok(Self {
            entry: entry.to_owned(),
            files: sources,
        })
    }

    /// Sources that crossed a process boundary, held to the limits [`Sources::read`] applies.
    pub fn new(entry: String, files: Vec<(String, Vec<u8>)>) -> Result<Self, Diagnostic> {
        if files.len() > MAX_SOURCE_FILES {
            return Err(too_many_files());
        }
        let mut remaining = MAX_SOURCE_BYTES;
        let mut sources = Vec::with_capacity(files.len());
        for (name, bytes) in files {
            remaining = remaining
                .checked_sub(bytes.len())
                .ok_or_else(|| too_large(&name))?;
            let source = String::from_utf8(bytes).map_err(|_| not_utf8(&name))?;
            sources.push((name, source));
        }
        if !sources.iter().any(|(name, _)| *name == entry) {
            return Err(missing_entry(&entry));
        }
        Ok(Self {
            entry,
            files: sources,
        })
    }

    pub fn entry(&self) -> &str {
        &self.entry
    }

    /// Each file's name, relative to the package folder, and its text.
    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files
            .iter()
            .map(|(name, source)| (name.as_str(), source.as_str()))
    }
}

fn missing_entry(entry: &str) -> Diagnostic {
    Diagnostic::new("missing-entry", "is not a module in the package folder")
        .file(entry)
        .hint(format!(
            "Create `{}` exporting `search`, or point `dupin.main` in package.json at the module that does.",
            crate::DEFAULT_ENTRY
        ))
}

fn too_many_files() -> Diagnostic {
    Diagnostic::new(
        "too-many-files",
        format!("The package has more than {MAX_SOURCE_FILES} modules"),
    )
    .hint("Bundle the extension into fewer files with esbuild or Bun.")
}

fn too_large(name: &str) -> Diagnostic {
    Diagnostic::new(
        "package-too-large",
        format!(
            "The package's modules are larger than {} MiB",
            MAX_SOURCE_BYTES / 1024 / 1024
        ),
    )
    .file(name)
    .hint("Bundle and minify the extension, and leave out code it doesn't run.")
}

fn not_utf8(name: &str) -> Diagnostic {
    Diagnostic::new("not-utf8", "is not UTF-8 text")
        .file(name)
        .hint("Save the file as UTF-8.")
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
            "Import package files with `./`, and bundle dependencies with esbuild or Bun before installing.",
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
        let hint = if is_module(&name) {
            "Check the path; imports are relative to the importing file."
        } else {
            "Write the file's extension in the import, such as `./util.js`."
        };
        Err(located(Diagnostic::new(
            "import-missing",
            format!("names `{name}`, which is not a module in the package"),
        ))
        .hint(hint))
    }
}
