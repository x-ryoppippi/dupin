//! The backdrop's WGSL: the built-in shader or a user's file, wrapped in Dupin's prelude and
//! entry point and validated before it reaches the GPU, so a broken file is a message in the
//! panel instead of a crash.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dupin_core::Backdrop;

mod metal;

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
        let result = match read_at_most(path) {
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

/// Flow, wrapped but not run through Metal: the tests check it compiles, and Dupin draws it from
/// launch, where a Metal compile would add to the time before the panel can show.
/// The file's text, read only as far as one byte past `MAX_LEN`: `compile` refuses anything longer,
/// and a backdrop pointed at a huge file shouldn't load it all first.
fn read_at_most(path: &Path) -> std::io::Result<String> {
    let mut text = String::new();
    std::io::Read::read_to_string(
        &mut std::io::Read::take(std::fs::File::open(path)?, MAX_LEN as u64 + 1),
        &mut text,
    )?;
    Ok(text)
}

fn builtin() -> Arc<str> {
    wrap(FLOW).0.into()
}

/// `user` between Dupin's prelude and entry point, and the line in the result that `user` starts on.
fn wrap(user: &str) -> (String, u32) {
    let head = format!("{}\n{PRELUDE}\n", rune::render::CLIP_WGSL);
    let first_user_line = head.lines().count() as u32 + 1;
    (format!("{head}{user}\n{ENTRY}"), first_user_line)
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

/// The longest backdrop shader Dupin checks. Real ones are a few KB; the limit bounds how deeply
/// a file can nest, which naga's parser and writer recurse on.
const MAX_LEN: usize = 32 * 1024;
/// The stack for naga's work on a backdrop: deep enough for anything within `MAX_LEN`, which on
/// the main thread overflows naga's recursion and aborts. Only touched pages are committed.
const DEEP_STACK: usize = 1 << 30;

/// Run `f` on a thread with `DEEP_STACK`: anything that has naga parse, validate or translate a
/// user's shader, whether Dupin's check or Rune making the module and pipeline from it.
pub fn deep<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("backdrop shader".into())
            .stack_size(DEEP_STACK)
            .spawn_scoped(scope, f)
            .expect("a thread for the backdrop shader");
        worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// `user` (which defines `fn backdrop(p: vec2f, b: Backdrop) -> vec4f`) wrapped into a module
/// with Dupin's uniforms and entry points, validated. An error reads `line:column: message`,
/// counted in `user`.
pub fn compile(user: &str) -> Result<Arc<str>, String> {
    if user.len() > MAX_LEN {
        return Err(format!(
            " is {} KB; a backdrop shader can be at most {} KB",
            user.len().div_ceil(1024),
            MAX_LEN / 1024
        ));
    }
    deep(|| check(user))
}

fn check(user: &str) -> Result<Arc<str>, String> {
    let (full, first_user_line) = wrap(user);
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
    metal::check(&module, &info)?;
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
        compile(FLOW).unwrap();
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

    /// Shaders naga accepts, and writes MSL for, that Metal itself refuses; each would end Dupin
    /// in Rune's backend. A function whose locals pass the GPU's stack is refused too, but where
    /// that limit falls depends on the GPU (a CI runner's virtual one takes 256 KB), so it has no
    /// case here.
    #[test]
    fn what_metal_refuses_is_an_error() {
        if !metal::available() {
            return;
        }
        let sum = vec!["p.x"; 2000].join(" + ");
        let cases = [
            (
                "a naga MSL bug",
                "let u = u32(p.x);\n    return vec4f(f32(dot4U8Packed(u, u)), 0.0, 0.0, 1.0);".to_owned(),
            ),
            (
                "a macro of Metal's",
                "let MAXBFLOAT = p.x;\n    return vec4f(MAXBFLOAT, 0.0, 0.0, 1.0);".to_owned(),
            ),
            (
                "a C keyword",
                "let _Generic = p.x;\n    return vec4f(_Generic, 0.0, 0.0, 1.0);".to_owned(),
            ),
            (
                "nesting past the compiler's",
                format!("return vec4f({sum}, 0.0, 0.0, 1.0);"),
            ),
        ];
        for (what, body) in cases {
            let e = compile(&format!(
                "fn backdrop(p: vec2f, b: Backdrop) -> vec4f {{\n    {body}\n}}\n"
            ));
            assert!(e.as_ref().is_err_and(|e| e.contains("Metal")), "{what}: {e:?}");
        }
        let e = compile(
            "fn ulong() -> f32 { return 1.0; }\nfn backdrop(p: vec2f, b: Backdrop) -> vec4f {\n    return vec4f(ulong());\n}\n",
        );
        assert!(
            e.as_ref().is_err_and(|e| e.contains("Metal")),
            "a type of Metal's: {e:?}"
        );
    }

    /// The deepest expression a file within the limit can hold is an error, not a stack overflow.
    #[test]
    fn the_longest_chain_within_the_limit_is_checked() {
        let head = "fn backdrop(p: vec2f, b: Backdrop) -> vec4f {\n    let x = p.x;\n    return vec4f(x";
        let n = (MAX_LEN - head.len() - 8) / 2;
        let user = format!("{head}{});\n}}", "+x".repeat(n));
        assert!(user.len() <= MAX_LEN);
        let checked = compile(&user);
        if metal::available() {
            assert!(checked.is_err_and(|e| e.contains("Metal")));
        }
    }

    #[test]
    fn a_file_past_the_limit_is_an_error() {
        let e = compile(&" ".repeat(MAX_LEN + 1)).unwrap_err();
        assert!(e.contains("at most"), "{e}");
    }

    #[test]
    fn a_missing_entry_function_is_an_error() {
        assert!(compile("fn other() {}").is_err());
    }
}
