//! The `exec` capability: named profiles that bind a user-selected executable, or an installed
//! script command (`@script:<name>`), to an argument template with typed inputs.
//!
//! A guest supplies only input values. [`Profile::bind`] validates them and renders argv without
//! a shell, so a value can never become another argument, a redirection or a command. The host
//! runs the result through a [`Spawn`] implementation, the script runner, which owns the process
//! group, timeout, cancellation and output bounds.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_TIMEOUT: Duration = Duration::from_secs(300);
pub const DEFAULT_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_ARGS: usize = 64;
const MAX_ARG_BYTES: usize = 64 * 1024;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_NAME_BYTES: usize = 64;
const SCRIPT_PREFIX: &str = "@script:";
const MINIMAL_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
const PASSED_VARIABLES: [&str; 4] = ["HOME", "TMPDIR", "LANG", "USER"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecError {
    InvalidProfile(String),
    UnknownInput(String),
    InvalidInput { input: String, problem: String },
    Unavailable(String),
    Failed(String),
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProfile(problem) => write!(f, "Invalid command profile: {problem}."),
            Self::UnknownInput(input) => write!(f, "The command has no input named {input:?}."),
            Self::InvalidInput { input, problem } => write!(f, "{input}: {problem}."),
            Self::Unavailable(problem) | Self::Failed(problem) => f.write_str(problem),
        }
    }
}

impl std::error::Error for ExecError {}

fn invalid(problem: impl Into<String>) -> ExecError {
    ExecError::InvalidProfile(problem.into())
}

/// Whether `text` shows as itself in the consent text: control, line-separator and invisible
/// formatting characters such as bidi overrides could forge or reorder the lines a user approves.
fn displayable(text: &str) -> bool {
    !text.chars().any(|ch| {
        ch.is_control()
            || matches!(
                ch,
                '\u{ad}'
                    | '\u{600}'..='\u{605}'
                    | '\u{61c}'
                    | '\u{6dd}'
                    | '\u{70f}'
                    | '\u{890}'..='\u{891}'
                    | '\u{8e2}'
                    | '\u{180e}'
                    | '\u{200b}'..='\u{200f}'
                    | '\u{2028}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}'
                    | '\u{feff}'
                    | '\u{fff9}'..='\u{fffb}'
                    | '\u{110bd}'
                    | '\u{110cd}'
                    | '\u{13430}'..='\u{1343f}'
                    | '\u{1bca0}'..='\u{1bca3}'
                    | '\u{1d173}'..='\u{1d17a}'
                    | '\u{e0001}'
                    | '\u{e0020}'..='\u{e007f}'
            )
    })
}

fn valid_name(name: &str) -> bool {
    (1..=MAX_NAME_BYTES).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_'))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Executable(PathBuf),
    Script(String),
}

impl Target {
    /// Parses `@script:<filename>` or an absolute executable path.
    pub fn parse(text: &str) -> Result<Self, ExecError> {
        if !displayable(text) {
            return Err(invalid(
                "the target contains control or invisible formatting characters",
            ));
        }
        if let Some(name) = text.strip_prefix(SCRIPT_PREFIX) {
            if name.is_empty() || name.len() > 255 || name.starts_with('.') || name.contains('/') {
                return Err(invalid("a script target names one file in the scripts folder"));
            }
            return Ok(Self::Script(name.into()));
        }
        let path = PathBuf::from(text);
        if !path.is_absolute() {
            return Err(invalid("the executable path must be absolute"));
        }
        Ok(Self::Executable(path))
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Executable(path) => path.display().fmt(f),
            Self::Script(name) => write!(f, "{SCRIPT_PREFIX}{name}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputKind {
    /// Free text. A value that starts an argument may not start with `-` unless `option_like`
    /// is set or the template places it after a literal `--`, so it can't be read as an option.
    Text {
        max_bytes: usize,
        multiline: bool,
        option_like: bool,
    },
    /// A negative value that starts an argument is refused for the same reason as text.
    Integer {
        min: i64,
        max: i64,
    },
    Choice(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub kind: InputKind,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Integer(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Arg {
    Literal(String),
    Input {
        prefix: String,
        name: String,
        suffix: String,
    },
}

fn parse_arg(text: &str, inputs: &BTreeMap<String, Input>) -> Result<Arg, ExecError> {
    if !displayable(text) {
        return Err(invalid(format!(
            "{text:?} contains control or invisible formatting characters"
        )));
    }
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut name = None;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let out = if name.is_some() { &mut suffix } else { &mut prefix };
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' if name.is_none() => {
                let mut placeholder = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(ch) => placeholder.push(ch),
                        None => return Err(invalid(format!("unclosed placeholder in {text:?}"))),
                    }
                }
                if !inputs.contains_key(&placeholder) {
                    return Err(invalid(format!("{{{placeholder}}} is not a declared input")));
                }
                name = Some(placeholder);
            }
            '{' => return Err(invalid(format!("{text:?} has more than one placeholder"))),
            '}' => {
                return Err(invalid(format!(
                    "unmatched }} in {text:?}; write }}}} for a brace"
                )));
            }
            ch => out.push(ch),
        }
    }
    Ok(match name {
        Some(name) => Arg::Input { prefix, name, suffix },
        None => Arg::Literal(prefix),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The working folder for an executable; the user's home folder when `None`. Script targets
    /// always run in the home folder, as they do from the launcher.
    pub working_directory: Option<PathBuf>,
    pub timeout: Duration,
    pub output_bytes: usize,
    /// Whether the guest receives stdout and stderr. Without it the guest learns only the exit.
    pub return_output: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            working_directory: None,
            timeout: DEFAULT_TIMEOUT,
            output_bytes: DEFAULT_OUTPUT_BYTES,
            return_output: false,
        }
    }
}

/// A user-approved command an extension may run with values for its declared inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    name: String,
    target: Target,
    args: Vec<Arg>,
    inputs: BTreeMap<String, Input>,
    options: Options,
}

impl Profile {
    pub fn new(
        name: &str,
        target: Target,
        template: &[&str],
        inputs: BTreeMap<String, Input>,
        options: Options,
    ) -> Result<Self, ExecError> {
        if !valid_name(name) {
            return Err(invalid(
                "names use 1–64 lowercase letters, digits, hyphens or underscores",
            ));
        }
        if template.len() > MAX_ARGS {
            return Err(invalid("at most 64 arguments are allowed"));
        }
        for (input, spec) in &inputs {
            if !valid_name(input) {
                return Err(invalid(format!("{input:?} is not a valid input name")));
            }
            match &spec.kind {
                InputKind::Text { max_bytes, .. } if *max_bytes == 0 || *max_bytes > MAX_TEXT_BYTES => {
                    return Err(invalid(format!("{input} must allow 1 to 16384 bytes")));
                }
                InputKind::Integer { min, max } if min > max => {
                    return Err(invalid(format!("{input} has an empty range")));
                }
                InputKind::Choice(choices)
                    if choices.is_empty() || !choices.iter().all(|choice| displayable(choice)) =>
                {
                    return Err(invalid(format!(
                        "{input} needs choices without control or invisible formatting characters"
                    )));
                }
                _ => {}
            }
        }
        let args = template
            .iter()
            .map(|arg| parse_arg(arg, &inputs))
            .collect::<Result<Vec<_>, _>>()?;
        let mut after_separator = false;
        for pair in args.windows(2) {
            after_separator |= matches!(&pair[0], Arg::Literal(text) if text == "--");
            if let [Arg::Literal(option), Arg::Input { prefix, name, .. }] = pair
                && !after_separator
                && option.starts_with('-')
                && option != "-"
                && option != "--"
                && !option.contains('=')
                && prefix.is_empty()
                && !inputs[name].required
            {
                return Err(invalid(format!(
                    "{name} is optional, so it cannot follow {option:?} as its own argument: leaving it out \
                     would hand the next argument to that option; write them as one argument, such as \
                     \"{option}={{{name}}}\", or make {name} required"
                )));
            }
        }
        if let Some(unused) = inputs.keys().find(|input| {
            !args
                .iter()
                .any(|arg| matches!(arg, Arg::Input { name, .. } if name == *input))
        }) {
            return Err(invalid(format!("{unused} is not used by the arguments")));
        }
        if options.timeout.is_zero() || options.timeout > MAX_TIMEOUT {
            return Err(invalid("the timeout must be between zero and five minutes"));
        }
        if options.output_bytes > MAX_OUTPUT_BYTES {
            return Err(invalid("output is limited to 1 MiB"));
        }
        match (&target, &options.working_directory) {
            (Target::Script(_), Some(_)) => {
                return Err(invalid("script commands run in your home folder"));
            }
            (_, Some(directory)) if !directory.is_absolute() => {
                return Err(invalid("the working folder must be absolute"));
            }
            (_, Some(directory)) if !displayable(&directory.to_string_lossy()) => {
                return Err(invalid(
                    "the working folder contains control or invisible formatting characters",
                ));
            }
            _ => {}
        }
        Ok(Self {
            name: name.into(),
            target,
            args,
            inputs,
            options,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn options(&self) -> &Options {
        &self.options
    }

    /// Validates `values` against the declared inputs and renders argv. An optional input
    /// without a value removes the whole argument that holds its placeholder.
    pub fn bind(&self, values: &BTreeMap<String, Value>) -> Result<Vec<String>, ExecError> {
        if let Some(unknown) = values.keys().find(|name| !self.inputs.contains_key(*name)) {
            return Err(ExecError::UnknownInput(unknown.clone()));
        }
        let mut argv = Vec::with_capacity(self.args.len());
        let mut after_separator = false;
        for arg in &self.args {
            match arg {
                Arg::Literal(text) => {
                    after_separator |= text == "--";
                    argv.push(text.clone());
                }
                Arg::Input { prefix, name, suffix } => {
                    let input = &self.inputs[name];
                    let Some(value) = values.get(name) else {
                        if input.required {
                            return Err(problem(name, "a value is required"));
                        }
                        continue;
                    };
                    let starts_argument = prefix.is_empty() && !after_separator;
                    let value = render(name, input, value, starts_argument)?;
                    argv.push(format!("{prefix}{value}{suffix}"));
                }
            }
        }
        if argv.iter().map(String::len).sum::<usize>() > MAX_ARG_BYTES {
            return Err(problem("arguments", "longer than 64 KiB in total"));
        }
        Ok(argv)
    }

    /// Exactly what the profile runs and what that implies, for the consent UI.
    pub fn describe(&self) -> String {
        let program = match &self.target {
            Target::Executable(path) => crate::display_path(path),
            Target::Script(name) => format!("your script command {name:?}"),
        };
        let args: Vec<_> = self
            .args
            .iter()
            .map(|arg| match arg {
                Arg::Literal(text) => quote(text),
                Arg::Input { prefix, name, suffix } => {
                    let input = &self.inputs[name];
                    let optional = if input.required { "" } else { ", optional" };
                    let kind = match &input.kind {
                        InputKind::Text {
                            max_bytes,
                            multiline,
                            option_like,
                        } => {
                            let mut kind = format!("text up to {max_bytes} bytes");
                            if *multiline {
                                kind.push_str(" on one or more lines");
                            }
                            if *option_like {
                                kind.push_str(", which may start with - and act as an option");
                            }
                            kind
                        }
                        InputKind::Integer { min, max } => format!("whole number {min}–{max}"),
                        InputKind::Choice(choices) => format!(
                            "one of {}",
                            choices
                                .iter()
                                .map(|choice| quote(choice))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    };
                    let placeholder = format!("{prefix}‹{name}: {kind}{optional}›{suffix}");
                    if quote(&format!("{prefix}{suffix}x")).starts_with('\'') {
                        format!("'{placeholder}'")
                    } else {
                        placeholder
                    }
                }
            })
            .collect();
        let mut lines = vec![if args.is_empty() {
            format!("Runs {program} with no arguments.")
        } else {
            format!("Runs {program} with arguments: {}", args.join(" "))
        }];
        lines.push(match (&self.target, &self.options.working_directory) {
            (Target::Script(_), _) => {
                "Runs in your home folder with Dupin's own environment, as when you run the script from Dupin."
                    .into()
            }
            (_, Some(directory)) => format!(
                "Runs in {} with only PATH, HOME, TMPDIR, LANG and USER set.",
                crate::display_path(directory)
            ),
            (_, None) => "Runs in your home folder with only PATH, HOME, TMPDIR, LANG and USER set.".into(),
        });
        lines.push(
            "It runs with your full user access and can use its own credentials, configuration and hooks."
                .into(),
        );
        lines.push(format!(
            "Dupin stops it and its child processes after {} s.",
            self.options.timeout.as_secs_f64()
        ));
        lines.push(if self.options.return_output {
            format!(
                "The extension receives the exit status and up to {} KiB of its output.",
                self.options.output_bytes.div_ceil(1024)
            )
        } else {
            "The extension receives only the exit status, not the output.".into()
        });
        lines.join("\n")
    }

    /// Binds `values` and resolves the target into a launch the host can spawn.
    pub fn prepare(
        &self,
        values: &BTreeMap<String, Value>,
        scripts: &impl Scripts,
    ) -> Result<Launch, ExecError> {
        let args = self.bind(values)?;
        let (program, environment) = match &self.target {
            Target::Script(name) => (
                scripts.resolve(name, &args).map_err(ExecError::Unavailable)?,
                Environment::Inherited,
            ),
            Target::Executable(path) => {
                let metadata = std::fs::metadata(path)
                    .map_err(|error| ExecError::Unavailable(format!("{}: {error}", path.display())))?;
                if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
                    return Err(ExecError::Unavailable(format!(
                        "{} is not an executable file",
                        path.display()
                    )));
                }
                (path.clone(), Environment::Minimal(minimal_environment()))
            }
        };
        Ok(Launch {
            program,
            args,
            working_directory: self.options.working_directory.clone(),
            environment,
            timeout: self.options.timeout,
            output_bytes: if self.options.return_output {
                self.options.output_bytes
            } else {
                0
            },
        })
    }
}

fn problem(input: &str, problem: &str) -> ExecError {
    ExecError::InvalidInput {
        input: input.into(),
        problem: problem.into(),
    }
}

fn render(name: &str, input: &Input, value: &Value, starts_argument: bool) -> Result<String, ExecError> {
    match (&input.kind, value) {
        (
            InputKind::Text {
                max_bytes,
                multiline,
                option_like,
            },
            Value::Text(text),
        ) => {
            if text.len() > *max_bytes {
                return Err(problem(name, &format!("longer than {max_bytes} bytes")));
            }
            if text
                .chars()
                .any(|ch| ch.is_control() && ch != '\t' && !(*multiline && ch == '\n'))
            {
                return Err(problem(name, "contains control characters"));
            }
            if starts_argument && !option_like && text.starts_with('-') {
                return Err(problem(name, "cannot start with -"));
            }
            Ok(text.clone())
        }
        (InputKind::Integer { min, max }, Value::Integer(number)) => {
            if number < min || number > max {
                return Err(problem(name, &format!("must be between {min} and {max}")));
            }
            if starts_argument && *number < 0 {
                return Err(problem(
                    name,
                    "cannot be negative here, where it would read as an option",
                ));
            }
            Ok(number.to_string())
        }
        (InputKind::Choice(choices), Value::Text(text)) => choices
            .iter()
            .find(|choice| *choice == text)
            .cloned()
            .ok_or_else(|| problem(name, "is not one of the allowed choices")),
        (InputKind::Integer { .. }, _) => Err(problem(name, "must be a whole number")),
        _ => Err(problem(name, "must be text")),
    }
}

fn quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_alphanumeric() || "-_./:=,+@%".contains(ch))
    {
        text.into()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// The variables an executable profile receives: a fixed system PATH and the user's own HOME,
/// TMPDIR, LANG and USER. Nothing else from Dupin's environment is passed on.
pub fn minimal_environment() -> Vec<(String, OsString)> {
    let mut environment = vec![("PATH".to_owned(), OsString::from(MINIMAL_PATH))];
    environment.extend(PASSED_VARIABLES.iter().filter_map(|name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(|value| ((*name).to_owned(), value))
    }));
    environment
}

/// Resolves `@script:<name>` to an installed script command after checking `args` against its
/// declared arguments.
pub trait Scripts {
    fn resolve(&self, name: &str, args: &[String]) -> Result<PathBuf, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Environment {
    Minimal(Vec<(String, OsString)>),
    Inherited,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub environment: Environment,
    pub timeout: Duration,
    /// The most stdout, and separately stderr, to keep. Zero discards both.
    pub output_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Captured {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub exit: Exit,
    pub output: Captured,
}

/// Runs a launch to completion on the calling thread, in its own process group with stdin
/// closed, killing the group on timeout or cancellation and when the program exits.
pub trait Spawn {
    fn spawn(&self, launch: &Launch, cancel: &AtomicBool) -> Result<Completion, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub exit: Exit,
    /// Present only when the profile's grant returns output to the guest.
    pub output: Option<Output>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

/// Prepares and runs `profile` with `values`, returning only what its grant lets the guest see.
pub fn run(
    profile: &Profile,
    values: &BTreeMap<String, Value>,
    scripts: &impl Scripts,
    spawner: &impl Spawn,
    cancel: &AtomicBool,
) -> Result<Outcome, ExecError> {
    let launch = profile.prepare(values, scripts)?;
    let completion = spawner.spawn(&launch, cancel).map_err(ExecError::Failed)?;
    let output = profile.options.return_output.then(|| {
        let limit = launch.output_bytes;
        let mut captured = completion.output;
        let truncated = captured.truncated || captured.stdout.len() > limit || captured.stderr.len() > limit;
        captured.stdout.truncate(limit);
        captured.stderr.truncate(limit);
        Output {
            stdout: String::from_utf8_lossy(&captured.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&captured.stderr).into_owned(),
            truncated,
        }
    });
    Ok(Outcome {
        exit: completion.exit,
        output,
    })
}

#[cfg(test)]
mod tests;
