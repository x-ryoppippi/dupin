//! The backdrop's WGSL: the built-in shader or a user's file, wrapped in Dupin's prelude and
//! entry point and validated before it reaches the GPU, so a broken file is a message in the
//! panel instead of a crash.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dupin_core::Backdrop;

const FLOW: &str = include_str!("shaders/flow.wgsl");
const PRELUDE: &str = include_str!("shaders/prelude.wgsl");
const ENTRY: &str = include_str!("shaders/entry.wgsl");

/// The shaders in `examples/shaders`, for Settings to copy beside config.toml: `(name, title,
/// source)`.
pub const EXAMPLES: [(&str, &str, &str); 4] = [
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
    /// The file's text when it was last read, so a summon compiles it only once it changes.
    read: Option<String>,
    module: Option<Arc<str>>,
    /// Why the picked file isn't what's drawing, until a version of it is.
    problem: Option<String>,
}

impl Source {
    /// The source `pick` names. A file that fails falls back to the built-in shader.
    pub fn new(pick: &Backdrop) -> Source {
        let mut source = Source {
            file: None,
            read: None,
            module: None,
            problem: None,
        };
        match pick {
            Backdrop::Off => {}
            Backdrop::Flow => source.module = Some(builtin()),
            Backdrop::File(path) => {
                source.file = Some(resolve(path));
                source.refresh();
            }
        }
        source
    }

    /// The full module to draw, or `None` when the backdrop is off.
    pub fn module(&self) -> Option<&Arc<str>> {
        self.module.as_ref()
    }

    /// Why the picked file isn't what's drawing.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Read a user's file again, so editing it shows on the next summon, and compile it if its
    /// text changed. Returns the problem when it is new or different, for the caller to log.
    pub fn refresh(&mut self) -> Option<&str> {
        let path = self.file.as_ref()?;
        let result = match std::fs::read_to_string(path) {
            Ok(user) if self.read.as_ref() == Some(&user) => return None,
            Ok(user) => {
                let compiled = compile(&user).map_err(|e| format!("{}:{e}", path.display()));
                self.read = Some(user);
                compiled
            }
            Err(e) => {
                self.read = None;
                Err(format!("couldn't read {}: {e}", path.display()))
            }
        };
        let problem = match result {
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
        };
        let changed = problem.is_some() && problem != self.problem;
        self.problem = problem;
        self.problem.as_deref().filter(|_| changed)
    }
}

fn builtin() -> Arc<str> {
    compile(FLOW).expect("the built-in backdrop shader is valid")
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
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), caps)
        .validate(&module)
        .map_err(|e| format!("{}{}", at(e.location(&full)), chain(e.as_inner())))?;
    let span_at = |span: naga::Span| at(span.is_defined().then(|| span.location(&full)));
    if let Some((h, o)) = module.overrides.iter().next() {
        let name = o.name.as_deref().unwrap_or("");
        return Err(format!(
            "{}`override {name}` isn't supported; use a `const`",
            span_at(module.overrides.get_span(h))
        ));
    }
    for (h, g) in module.global_variables.iter() {
        let name = g.name.as_deref().unwrap_or("");
        if g.binding.is_some() && name != "dupin_u" {
            return Err(format!(
                "{}`{name}` binds a resource Dupin doesn't provide; a backdrop reads only `b`",
                span_at(module.global_variables.get_span(h))
            ));
        }
    }
    metal(&module, &info)?;
    Ok(full.into())
}

/// Translate both entry points to MSL as Rune's Metal backend will, with `dupin_u` in buffer
/// 0: Rune panics on a shader it can't translate, so it has to be caught here.
fn metal(module: &naga::Module, info: &naga::valid::ModuleInfo) -> Result<(), String> {
    use naga::back::msl;
    use naga::proc::{BoundsCheckPolicies, BoundsCheckPolicy};
    let mut resources = msl::EntryPointResources::default();
    resources.resources.insert(
        naga::ResourceBinding { group: 0, binding: 0 },
        msl::BindTarget {
            buffer: Some(0),
            ..Default::default()
        },
    );
    for (stage, entry) in [
        (naga::ShaderStage::Vertex, "dupin_vs"),
        (naga::ShaderStage::Fragment, "dupin_fs"),
    ] {
        let options = msl::Options {
            lang_version: (3, 0),
            per_entry_point_map: [(entry.to_owned(), resources.clone())].into(),
            bounds_check_policies: BoundsCheckPolicies {
                index: BoundsCheckPolicy::Restrict,
                buffer: BoundsCheckPolicy::Restrict,
                image_load: BoundsCheckPolicy::Restrict,
                binding_array: BoundsCheckPolicy::Unchecked,
            },
            ..Default::default()
        };
        let pipeline = msl::PipelineOptions {
            entry_point: Some((stage, entry.to_owned())),
            ..Default::default()
        };
        msl::write_string(module, info, &options, &pipeline)
            .map_err(|e| format!(" can't be compiled for Metal: {e}"))?;
    }
    Ok(())
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
    fn a_file_that_fails_keeps_its_problem_and_is_tried_again() {
        let dir = std::env::temp_dir().join(format!("dupin-shader-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("later.wgsl");
        let _ = std::fs::remove_file(&path);
        let mut source = Source::new(&Backdrop::File(path.to_string_lossy().into_owned()));
        assert!(source.problem().is_some_and(|p| p.contains("couldn't read")));
        assert!(source.module().is_some(), "falls back to flow");
        assert_eq!(source.refresh(), None, "an unchanged problem isn't logged again");
        assert!(source.problem().is_some(), "still reported on the next summon");
        std::fs::write(&path, FLOW).unwrap();
        source.refresh();
        assert_eq!(source.problem(), None);

        let away = dir.join("away.wgsl");
        std::fs::rename(&path, &away).unwrap();
        assert!(source.refresh().is_some());
        std::fs::rename(&away, &path).unwrap();
        source.refresh();
        assert_eq!(source.problem(), None, "the same file, moved back, works again");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_override_is_an_error() {
        let e = compile("override k: f32 = 1.0;\nfn backdrop(p: vec2f, b: Backdrop) -> vec4f {\n    return vec4f(k);\n}\n")
            .unwrap_err();
        assert!(e.starts_with("1:") && e.contains("override"), "{e}");
    }

    #[test]
    fn a_binding_dupin_doesnt_provide_is_an_error() {
        let e = compile(
            "@group(0) @binding(1) var<uniform> mine: vec4f;\nfn backdrop(p: vec2f, b: Backdrop) -> vec4f {\n    return mine;\n}\n",
        )
        .unwrap_err();
        assert!(e.starts_with("1:") && e.contains("`mine`"), "{e}");
    }

    #[test]
    fn a_missing_entry_function_is_an_error() {
        assert!(compile("fn other() {}").is_err());
    }
}
