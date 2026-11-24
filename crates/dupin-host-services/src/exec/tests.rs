use super::*;
use std::cell::RefCell;

fn text(max_bytes: usize) -> Input {
    Input {
        kind: InputKind::Text {
            max_bytes,
            multiline: false,
            option_like: false,
        },
        required: true,
    }
}

fn inputs(entries: impl IntoIterator<Item = (&'static str, Input)>) -> BTreeMap<String, Input> {
    entries
        .into_iter()
        .map(|(name, input)| (name.to_owned(), input))
        .collect()
}

fn values(entries: impl IntoIterator<Item = (&'static str, Value)>) -> BTreeMap<String, Value> {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn grep() -> Profile {
    Profile::new(
        "grep",
        Target::parse("/usr/bin/grep").unwrap(),
        &["--max-count={count}", "-e", "{pattern}", "{path}"],
        inputs([
            ("pattern", text(64)),
            (
                "count",
                Input {
                    kind: InputKind::Integer { min: 1, max: 50 },
                    required: true,
                },
            ),
            (
                "path",
                Input {
                    kind: InputKind::Choice(vec!["notes.txt".into(), "todo.txt".into()]),
                    required: false,
                },
            ),
        ]),
        Options::default(),
    )
    .unwrap()
}

#[test]
fn values_render_into_argv_without_a_shell() {
    let profile = grep();
    let argv = profile
        .bind(&values([
            (
                "pattern",
                Value::Text("a b; rm -rf ~ $(touch x) `id` | cat > out".into()),
            ),
            ("count", Value::Integer(5)),
            ("path", Value::Text("todo.txt".into())),
        ]))
        .unwrap();
    assert_eq!(
        argv,
        [
            "--max-count=5",
            "-e",
            "a b; rm -rf ~ $(touch x) `id` | cat > out",
            "todo.txt"
        ]
    );
    let argv = profile
        .bind(&values([
            ("pattern", Value::Text("x".into())),
            ("count", Value::Integer(1)),
        ]))
        .unwrap();
    assert_eq!(argv, ["--max-count=1", "-e", "x"]);
}

#[test]
fn argument_injection_attempts_are_refused() {
    let profile = grep();
    let base = |pattern: &str| {
        values([
            ("pattern", Value::Text(pattern.into())),
            ("count", Value::Integer(5)),
        ])
    };
    for (values, expected) in [
        (base("-rf"), "cannot start with -"),
        (base("--exec=evil"), "cannot start with -"),
        (base("a\0b"), "control characters"),
        (base("line\nbreak"), "control characters"),
        (base("\u{1b}[31m"), "control characters"),
        (base(&"x".repeat(65)), "longer than 64 bytes"),
        (
            values([
                ("pattern", Value::Text("x".into())),
                ("count", Value::Integer(51)),
            ]),
            "between 1 and 50",
        ),
        (
            values([
                ("pattern", Value::Text("x".into())),
                ("count", Value::Text("5".into())),
            ]),
            "whole number",
        ),
        (
            values([("pattern", Value::Integer(1)), ("count", Value::Integer(5))]),
            "must be text",
        ),
        (
            values([
                ("pattern", Value::Text("x".into())),
                ("count", Value::Integer(5)),
                ("path", Value::Text("../../etc/passwd".into())),
            ]),
            "allowed choices",
        ),
        (values([("count", Value::Integer(5))]), "a value is required"),
    ] {
        let error = profile.bind(&values).unwrap_err().to_string();
        assert!(error.contains(expected), "{values:?}: {error}");
    }
    let mut extra = base("x");
    extra.insert("shell".into(), Value::Text("sh -c id".into()));
    assert_eq!(profile.bind(&extra), Err(ExecError::UnknownInput("shell".into())));
}

#[test]
fn option_like_text_is_allowed_after_a_separator_or_a_prefix_or_when_declared() {
    let make = |template: &[&str], option_like| {
        Profile::new(
            "echo",
            Target::parse("/bin/echo").unwrap(),
            template,
            inputs([(
                "word",
                Input {
                    kind: InputKind::Text {
                        max_bytes: 16,
                        multiline: true,
                        option_like,
                    },
                    required: true,
                },
            )]),
            Options::default(),
        )
        .unwrap()
    };
    let word = values([("word", Value::Text("-n\nx".into()))]);
    assert!(make(&["{word}"], false).bind(&word).is_err());
    assert_eq!(
        make(&["--", "{word}"], false).bind(&word).unwrap(),
        ["--", "-n\nx"]
    );
    assert_eq!(
        make(&["--word={word}"], false).bind(&word).unwrap(),
        ["--word=-n\nx"]
    );
    assert_eq!(make(&["{word}"], true).bind(&word).unwrap(), ["-n\nx"]);
    assert_eq!(
        make(&["{{literal}}", "<{word}>"], false).bind(&word).unwrap(),
        ["{literal}", "<-n\nx>"]
    );
}

#[test]
fn invalid_profiles_are_refused() {
    let one = || inputs([("word", text(8))]);
    let target = || Target::parse("/bin/echo").unwrap();
    for (name, template, inputs, options, expected) in [
        ("Echo", vec!["{word}"], one(), Options::default(), "lowercase"),
        (
            "echo",
            vec!["{other}"],
            one(),
            Options::default(),
            "not a declared input",
        ),
        (
            "echo",
            vec!["{word}{word}"],
            one(),
            Options::default(),
            "more than one",
        ),
        ("echo", vec!["{word"], one(), Options::default(), "unclosed"),
        ("echo", vec!["a}"], one(), Options::default(), "unmatched"),
        (
            "echo",
            vec!["a\0", "{word}"],
            one(),
            Options::default(),
            "control",
        ),
        ("echo", vec!["plain"], one(), Options::default(), "not used"),
        (
            "echo",
            vec!["{word}"; 65],
            one(),
            Options::default(),
            "64 arguments",
        ),
        (
            "echo",
            vec!["{word}"],
            inputs([("word", text(MAX_TEXT_BYTES + 1))]),
            Options::default(),
            "16384",
        ),
        (
            "echo",
            vec!["{word}"],
            one(),
            Options {
                timeout: MAX_TIMEOUT + Duration::from_secs(1),
                ..Options::default()
            },
            "timeout",
        ),
        (
            "echo",
            vec!["{word}"],
            one(),
            Options {
                output_bytes: MAX_OUTPUT_BYTES + 1,
                ..Options::default()
            },
            "1 MiB",
        ),
        (
            "echo",
            vec!["{word}"],
            one(),
            Options {
                working_directory: Some("relative".into()),
                ..Options::default()
            },
            "absolute",
        ),
    ] {
        let error = Profile::new(name, target(), &template, inputs, options)
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{template:?}: {error}");
    }
    assert!(Target::parse("echo").is_err());
    for script in ["@script:", "@script:../escape", "@script:a/b", "@script:.hidden"] {
        assert!(Target::parse(script).is_err(), "{script}");
    }
    assert!(
        Profile::new(
            "script",
            Target::parse("@script:fixture.sh").unwrap(),
            &[],
            BTreeMap::new(),
            Options {
                working_directory: Some("/tmp".into()),
                ..Options::default()
            },
        )
        .is_err()
    );
}

#[test]
fn manifest_text_that_could_forge_or_reorder_the_consent_text_is_refused() {
    let target = || Target::parse("/bin/echo").unwrap();
    let forged = "x\nThe extension receives only the exit status, not the output.";
    for template in [
        vec![forged, "{word}"],
        vec!["{word}", "a\u{202e}txt.exe"],
        vec!["<\u{2066}{word}"],
        vec!["{word}\u{200b}"],
        vec!["\u{2028}", "{word}"],
    ] {
        let error = Profile::new(
            "echo",
            target(),
            &template,
            inputs([("word", text(8))]),
            Options::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("invisible formatting"), "{template:?}: {error}");
    }
    let choice = Input {
        kind: InputKind::Choice(vec!["a".into(), "b\nForged line".into()]),
        required: true,
    };
    assert!(
        Profile::new(
            "echo",
            target(),
            &["{word}"],
            inputs([("word", choice)]),
            Options::default()
        )
        .is_err()
    );
    assert!(Target::parse("/usr/bin/\u{202e}tac").is_err());
    let options = Options {
        working_directory: Some("/tmp/x\nForged line".into()),
        ..Options::default()
    };
    assert!(
        Profile::new(
            "echo",
            target(),
            &["{word}"],
            inputs([("word", text(8))]),
            options
        )
        .is_err()
    );
}

#[test]
fn an_optional_input_cannot_stand_alone_after_an_option_it_could_shift_into() {
    let make = |template: &[&str]| {
        Profile::new(
            "echo",
            Target::parse("/bin/echo").unwrap(),
            template,
            inputs([
                (
                    "out",
                    Input {
                        required: false,
                        ..text(64)
                    },
                ),
                ("input", text(64)),
            ]),
            Options::default(),
        )
    };
    for template in [
        &["-o", "{out}", "{input}"][..],
        &["--output", "{out}", "{input}"],
        &["{input}", "-o", "{out}"],
    ] {
        let error = make(template).unwrap_err().to_string();
        assert!(error.contains("out is optional"), "{template:?}: {error}");
    }
    for template in [
        &["--output={out}", "{input}"][..],
        &["-o{out}", "{input}"],
        &["-", "{out}", "{input}"],
        &["--", "-o", "{out}", "{input}"],
        &["{out}", "-o", "{input}"],
    ] {
        assert!(make(template).is_ok(), "{template:?}");
    }
    let profile = make(&["--output={out}", "{input}"]).unwrap();
    assert_eq!(
        profile
            .bind(&values([("input", Value::Text("notes.txt".into()))]))
            .unwrap(),
        ["notes.txt"]
    );
}

#[test]
fn negative_integers_are_refused_where_they_would_read_as_an_option() {
    let make = |template: &[&str]| {
        Profile::new(
            "nice",
            Target::parse("/usr/bin/nice").unwrap(),
            template,
            inputs([(
                "offset",
                Input {
                    kind: InputKind::Integer { min: -10, max: 10 },
                    required: true,
                },
            )]),
            Options::default(),
        )
        .unwrap()
    };
    let negative = values([("offset", Value::Integer(-1))]);
    let error = make(&["{offset}", "file"])
        .bind(&negative)
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot be negative"), "{error}");
    assert_eq!(
        make(&["{offset}", "file"])
            .bind(&values([("offset", Value::Integer(1))]))
            .unwrap(),
        ["1", "file"]
    );
    assert_eq!(make(&["--", "{offset}"]).bind(&negative).unwrap(), ["--", "-1"]);
    assert_eq!(
        make(&["--offset={offset}"]).bind(&negative).unwrap(),
        ["--offset=-1"]
    );
}

#[test]
fn the_description_shows_text_that_may_span_lines_or_act_as_an_option() {
    let description = Profile::new(
        "git",
        Target::parse("/usr/bin/git").unwrap(),
        &["-C", "/repo", "{sub}", "status", "--", "{message}"],
        inputs([
            (
                "sub",
                Input {
                    kind: InputKind::Text {
                        max_bytes: 128,
                        multiline: false,
                        option_like: true,
                    },
                    required: true,
                },
            ),
            (
                "message",
                Input {
                    kind: InputKind::Text {
                        max_bytes: 64,
                        multiline: true,
                        option_like: false,
                    },
                    required: true,
                },
            ),
        ]),
        Options::default(),
    )
    .unwrap()
    .describe();
    assert_eq!(
        description.lines().next().unwrap(),
        "Runs /usr/bin/git with arguments: -C /repo ‹sub: text up to 128 bytes, which may start with - and act as an option› status -- ‹message: text up to 64 bytes on one or more lines›"
    );
}

#[test]
fn the_description_says_exactly_what_runs_and_what_the_guest_receives() {
    let description = grep().describe();
    assert_eq!(
        description.lines().next().unwrap(),
        "Runs /usr/bin/grep with arguments: --max-count=‹count: whole number 1–50› -e ‹pattern: text up to 64 bytes› ‹path: one of notes.txt, todo.txt, optional›"
    );
    assert!(description.contains("only PATH, HOME, TMPDIR, LANG and USER"));
    assert!(description.contains("full user access"));
    assert!(description.contains("after 30 s"));
    assert!(description.contains("only the exit status"));

    let script = Profile::new(
        "notify",
        Target::parse("@script:notify.sh").unwrap(),
        &["hello world", "{message}"],
        inputs([("message", text(32))]),
        Options {
            return_output: true,
            output_bytes: 2048,
            ..Options::default()
        },
    )
    .unwrap()
    .describe();
    assert!(script.starts_with(
        "Runs your script command \"notify.sh\" with arguments: 'hello world' ‹message: text up to 32 bytes›"
    ));
    assert!(script.contains("Dupin's own environment"));
    assert!(script.contains("up to 2 KiB of its output"));
}

struct FakeScripts;

impl Scripts for FakeScripts {
    fn resolve(&self, name: &str, args: &[String]) -> Result<PathBuf, String> {
        match (name, args) {
            ("fixture.sh", [_]) => Ok(PathBuf::from("/scripts/fixture.sh")),
            ("fixture.sh", _) => Err("Too many arguments".into()),
            _ => Err(format!("Script {name:?} was not found in the scripts directory")),
        }
    }
}

struct Recorder {
    launches: RefCell<Vec<Launch>>,
    completion: Completion,
}

impl Spawn for Recorder {
    fn spawn(&self, launch: &Launch, _: &AtomicBool) -> Result<Completion, String> {
        self.launches.borrow_mut().push(launch.clone());
        Ok(self.completion.clone())
    }
}

fn recorder(stdout: &[u8]) -> Recorder {
    Recorder {
        launches: RefCell::default(),
        completion: Completion {
            exit: Exit::Code(0),
            output: Captured {
                stdout: stdout.to_vec(),
                stderr: b"warning".to_vec(),
                truncated: false,
            },
        },
    }
}

#[test]
fn script_targets_resolve_through_the_catalogue_and_inherit_the_runner_environment() {
    let profile = Profile::new(
        "fixture",
        Target::parse("@script:fixture.sh").unwrap(),
        &["{message}"],
        inputs([("message", text(32))]),
        Options::default(),
    )
    .unwrap();
    let spawner = recorder(b"ignored");
    let outcome = run(
        &profile,
        &values([("message", Value::Text("hello".into()))]),
        &FakeScripts,
        &spawner,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(outcome.output, None);
    let launch = spawner.launches.borrow()[0].clone();
    assert_eq!(launch.program, PathBuf::from("/scripts/fixture.sh"));
    assert_eq!(launch.args, ["hello"]);
    assert_eq!(launch.environment, Environment::Inherited);
    assert_eq!(launch.working_directory, None);
    assert_eq!(launch.output_bytes, 0);

    let missing = Profile::new(
        "missing",
        Target::parse("@script:missing").unwrap(),
        &[],
        BTreeMap::new(),
        Options::default(),
    )
    .unwrap();
    let error = run(
        &missing,
        &BTreeMap::new(),
        &FakeScripts,
        &spawner,
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.to_string().contains("not found"));
    assert_eq!(spawner.launches.borrow().len(), 1);
}

#[cfg(target_os = "macos")]
#[test]
fn output_reaches_the_guest_only_with_the_grant_flag_and_within_its_bound() {
    let make = |return_output| {
        Profile::new(
            "echo",
            Target::parse("/bin/echo").unwrap(),
            &["fixed"],
            BTreeMap::new(),
            Options {
                return_output,
                output_bytes: 4,
                working_directory: Some("/".into()),
                ..Options::default()
            },
        )
        .unwrap()
    };
    let spawner = recorder(b"0123456789");
    let hidden = run(
        &make(false),
        &BTreeMap::new(),
        &FakeScripts,
        &spawner,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(hidden.exit, Exit::Code(0));
    assert_eq!(hidden.output, None);
    let shown = run(
        &make(true),
        &BTreeMap::new(),
        &FakeScripts,
        &spawner,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        shown.output,
        Some(Output {
            stdout: "0123".into(),
            stderr: "warn".into(),
            truncated: true
        })
    );
    let launches = spawner.launches.borrow();
    assert_eq!(launches[0].output_bytes, 0);
    assert_eq!(launches[1].output_bytes, 4);
    assert_eq!(launches[1].working_directory, Some(PathBuf::from("/")));
    let Environment::Minimal(environment) = &launches[1].environment else {
        panic!("executables get a minimal environment");
    };
    assert_eq!(environment[0], ("PATH".into(), OsString::from(MINIMAL_PATH)));
    assert!(
        environment
            .iter()
            .all(|(name, _)| name == "PATH" || PASSED_VARIABLES.contains(&name.as_str()))
    );
}

#[test]
fn a_missing_or_non_executable_target_is_reported_before_spawning() {
    let temp = tempfile::tempdir().unwrap();
    let plain = temp.path().join("plain");
    std::fs::write(&plain, "#!/bin/sh\n").unwrap();
    let spawner = recorder(b"");
    for path in [plain, temp.path().join("missing"), temp.path().to_path_buf()] {
        let profile = Profile::new(
            "tool",
            Target::Executable(path),
            &[],
            BTreeMap::new(),
            Options::default(),
        )
        .unwrap();
        assert!(matches!(
            run(
                &profile,
                &BTreeMap::new(),
                &FakeScripts,
                &spawner,
                &AtomicBool::new(false)
            ),
            Err(ExecError::Unavailable(_))
        ));
    }
    assert!(spawner.launches.borrow().is_empty());
}
