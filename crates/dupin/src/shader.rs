//! The backdrop's WGSL: the built-in shader or a user's file, wrapped in Dupin's prelude and
//! entry point and validated before it reaches the GPU, so a broken file is a message in the
//! panel instead of a crash.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use dupin_core::Backdrop;

const FLOW: &str = include_str!("shaders/flow.wgsl");
const PRELUDE: &str = include_str!("shaders/prelude.wgsl");
const ENTRY: &str = include_str!("shaders/entry.wgsl");

/// The shaders in `examples/shaders`, for Settings to copy beside config.toml: `(name, title,
/// source)`.
pub const EXAMPLES: [(&str, &str, &str); 5] = [
    (
        "aurora",
        "Aurora",
        include_str!("../../../examples/shaders/aurora.wgsl"),
    ),
    (
        "embers",
        "Embers",
        include_str!("../../../examples/shaders/embers.wgsl"),
    ),
    (
        "snow",
        "Snow",
        include_str!("../../../examples/shaders/snow.wgsl"),
    ),
    (
        "ripple",
        "Ripple",
        include_str!("../../../examples/shaders/ripple.wgsl"),
    ),
    (
        "matrix",
        "Matrix",
        include_str!("../../../examples/shaders/matrix.wgsl"),
    ),
];

/// The folder beside config.toml that Settings lists shaders from, as config.toml names it.
const SHADERS: &str = "shaders";

/// Where Settings puts a copy of example `name`, as config.toml names it.
pub fn example_pick(name: &str) -> String {
    format!("{SHADERS}/{name}.wgsl")
}

pub fn shaders_dir() -> PathBuf {
    resolve(SHADERS)
}

/// The `.wgsl` files in the shaders folder other than copies of the examples, as config.toml
/// names them, sorted.
pub fn user_shaders() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(shaders_dir()) else {
        return Vec::new();
    };
    let mut picks: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|name| Some(name.strip_suffix(".wgsl")?.to_owned()))
        .filter(|stem| !EXAMPLES.iter().any(|(n, _, _)| n == stem))
        .map(|stem| example_pick(&stem))
        .collect();
    picks.sort();
    picks
}

/// Copy example `name` beside config.toml unless a copy, perhaps edited, is already there, and
/// return the `backdrop` value that picks it.
pub fn install_example(name: &str) -> std::io::Result<Backdrop> {
    let (_, _, src) = EXAMPLES
        .iter()
        .find(|(n, _, _)| *n == name)
        .ok_or_else(|| std::io::Error::other(format!("no example named {name}")))?;
    let pick = example_pick(name);
    let path = resolve(&pick);
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, src)?;
    }
    Ok(Backdrop::File(pick))
}

/// What the panel draws behind its content, as config.toml picked it.
pub struct Source {
    file: Option<PathBuf>,
    /// The file's modification time when it was last read, to read it again only once it changes.
    read_at: Option<SystemTime>,
    module: Option<Arc<str>>,
}

impl Source {
    /// The source `pick` names and, when it couldn't be used, why. A file that fails falls back
    /// to the built-in shader.
    pub fn new(pick: &Backdrop) -> (Source, Option<String>) {
        let mut source = Source {
            file: None,
            read_at: None,
            module: None,
        };
        let problem = match pick {
            Backdrop::Off => None,
            Backdrop::Flow => {
                source.module = Some(builtin());
                None
            }
            Backdrop::File(path) => {
                source.file = Some(resolve(path));
                source.reload()
            }
        };
        (source, problem)
    }

    /// The full module to draw, or `None` when the backdrop is off.
    pub fn module(&self) -> Option<&Arc<str>> {
        self.module.as_ref()
    }

    /// Read a user's file again if it changed since it was last read, so editing it shows on the
    /// next summon. Returns why the new version couldn't be used.
    pub fn refresh(&mut self) -> Option<String> {
        let path = self.file.as_ref()?;
        let changed = modified(path) != self.read_at;
        changed.then(|| self.reload()).flatten()
    }

    fn reload(&mut self) -> Option<String> {
        let path = self.file.as_ref()?;
        self.read_at = modified(path);
        let result = std::fs::read_to_string(path)
            .map_err(|e| format!("couldn't read {}: {e}", path.display()))
            .and_then(|user| compile(&user).map_err(|e| format!("{}:{e}", path.display())));
        match result {
            Ok(module) => {
                self.module = Some(module);
                None
            }
            Err(message) => {
                // Keep a version that worked, or fall back to the built-in one.
                if self.module.is_none() {
                    self.module = Some(builtin());
                }
                Some(format!("backdrop shader {message}"))
            }
        }
    }
}

fn builtin() -> Arc<str> {
    compile(FLOW).expect("the built-in backdrop shader is valid")
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// `~/x` under the home directory; a relative path beside config.toml.
pub fn resolve(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    let path = PathBuf::from(path);
    match crate::settings::path().as_deref().and_then(Path::parent) {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path,
    }
}

/// `user` (which defines `fn backdrop(p: vec2f, b: Backdrop) -> vec4f`) wrapped into a module
/// with Dupin's uniforms and entry points, validated. An error reads `line:column: message`,
/// counted in `user`.
pub fn compile(user: &str) -> Result<Arc<str>, String> {
    let head = format!("{}\n{PRELUDE}\n", rune::render::CLIP_WGSL);
    let full = format!("{head}{user}\n{ENTRY}");
    let first_user_line = head.lines().count() as u32 + 1;
    let at = |loc: Option<naga::SourceLocation>| match loc {
        Some(loc) if loc.line_number >= first_user_line => {
            format!(
                "{}:{}: ",
                loc.line_number - first_user_line + 1,
                loc.line_position
            )
        }
        _ => " ".to_owned(),
    };
    let module = naga::front::wgsl::parse_str(&full)
        .map_err(|e| format!("{}{}", at(e.location(&full)), e.message()))?;
    // Rune's clip helper unpacks half floats, which wgpu allows on Metal.
    let caps = naga::valid::Capabilities::default() | naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), caps)
        .validate(&module)
        .map_err(|e| format!("{}{}", at(e.location(&full)), chain(e.as_inner())))?;
    Ok(full.into())
}

/// An error and its causes, innermost last: naga's outer errors only name the function.
fn chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        out.push_str(": ");
        out.push_str(&c.to_string());
        cause = c.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_shader_compiles() {
        builtin();
    }

    #[test]
    fn every_example_compiles_and_settings_offers_it() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shaders");
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "wgsl"))
            .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        files.sort();
        let mut offered: Vec<String> = EXAMPLES.iter().map(|(n, _, _)| (*n).to_owned()).collect();
        offered.sort();
        assert_eq!(files, offered, "EXAMPLES and {} disagree", dir.display());
        for (name, _, src) in EXAMPLES {
            if let Err(e) = compile(src) {
                panic!("{name}.wgsl:{e}");
            }
        }
    }

    #[test]
    fn errors_point_into_the_users_file() {
        let e = compile("fn backdrop(p: vec2f, b: Backdrop) -> vec4f {\n    return nope;\n}\n").unwrap_err();
        assert!(e.starts_with("2:"), "{e}");
    }

    #[test]
    fn a_missing_entry_function_is_an_error() {
        assert!(compile("fn other() {}").is_err());
    }
}
