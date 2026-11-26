//! The bundled Remote package's exec profiles, run as the launcher runs them: built from its
//! `package.json` by the bridge and spawned by the script runner, against a fake `ssh` that
//! prints the argv and environment it received, so no test reaches a real host.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use dupin_extension_bridge::{Capabilities, Scope, exec_profile};
use dupin_host_services::exec;
use serde_json::Value;

use super::bundled;
use crate::scripts::runner::Runner;

/// What every ssh profile passes before the host: no terminal, no agent or X11 forwarding, no
/// prompts, a bounded connection and nothing from the user's configuration that would run or
/// forward something else.
const SSH: [&str; 15] = [
    "-n",
    "-T",
    "-a",
    "-x",
    "-oBatchMode=yes",
    "-oConnectTimeout=5",
    "-oServerAliveInterval=5",
    "-oServerAliveCountMax=2",
    "-oStrictHostKeyChecking=yes",
    "-oUpdateHostKeys=no",
    "-oClearAllForwardings=yes",
    "-oPermitLocalCommand=no",
    "-oRemoteCommand=none",
    "-oControlMaster=no",
    "-oLogLevel=ERROR",
];

struct NoScripts;

impl exec::Scripts for NoScripts {
    fn resolve(&self, name: &str, _: &[String]) -> Result<PathBuf, String> {
        Err(format!("{name} is not a script"))
    }
}

fn scopes() -> BTreeMap<String, Scope> {
    let path = bundled::directory().unwrap().join("remote/package.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    Capabilities::parse(&manifest["dupin"]["capabilities"])
        .unwrap()
        .scopes()
        .filter_map(|scope| match scope {
            Scope::Exec { name, .. } => Some((name.to_string(), scope.clone())),
            _ => None,
        })
        .collect()
}

fn fake_ssh(dir: &Path) -> PathBuf {
    let path = dir.join("ssh");
    std::fs::write(
        &path,
        "#!/bin/sh\nprintf '%s\\n' \"$@\"\nprintf 'SSH_AUTH_SOCK=%s\\n' \"${SSH_AUTH_SOCK-unset}\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

/// The argv and the agent socket the fake program received.
fn run(
    scope: &Scope,
    program: &Path,
    inputs: &[(&str, &str)],
) -> Result<(Vec<String>, String), exec::ExecError> {
    let profile = exec_profile(scope, program).unwrap();
    let values = inputs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), exec::Value::Text((*value).to_owned())))
        .collect();
    let runner = Runner::new(
        PathBuf::from("/"),
        Arc::new(|_, _| panic!("an extension command reached script history")),
    );
    let outcome = exec::run(&profile, &values, &NoScripts, &runner, &AtomicBool::new(false))?;
    assert_eq!(outcome.exit, exec::Exit::Code(0));
    let stdout = outcome.output.map(|output| output.stdout);
    let mut lines: Vec<String> = stdout.unwrap_or_default().lines().map(str::to_owned).collect();
    let socket = lines.pop().unwrap_or_default();
    Ok((lines, socket))
}

fn ssh(host: &str, command: &[&str]) -> Vec<String> {
    SSH.iter()
        .chain(&[host, "--"])
        .chain(command)
        .map(|arg| (*arg).to_owned())
        .collect()
}

#[test]
fn remote_runs_ssh_with_fixed_arguments_and_only_the_agent_socket_added() {
    let scopes = scopes();
    let dir = tempfile::tempdir().unwrap();
    let program = fake_ssh(dir.path());
    let agent = format!(
        "SSH_AUTH_SOCK={}",
        std::env::var("SSH_AUTH_SOCK")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unset".into())
    );
    let cases: [(&str, &[(&str, &str)], Vec<String>); 5] = [
        (
            "processes",
            &[("host", "web1")],
            ssh(
                "web1",
                &[
                    "ps", "-A", "-o", "pid=", "-o", "user=", "-o", "pcpu=", "-o", "pmem=", "-o", "etime=",
                    "-o", "args=",
                ],
            ),
        ),
        (
            "ports",
            &[("host", "web1")],
            ssh("web1", &["ss", "-H", "-l", "-n", "-t", "-u", "-p"]),
        ),
        (
            "journal",
            &[("host", "web1"), ("unit", "nginx.service"), ("lines", "50")],
            ssh(
                "web1",
                &[
                    "journalctl",
                    "--no-pager",
                    "-q",
                    "-o",
                    "short-iso",
                    "-n",
                    "50",
                    "-u",
                    "nginx.service",
                ],
            ),
        ),
        (
            "log-file",
            &[
                ("host", "deploy@web1"),
                ("path", "/var/log/app.log"),
                ("lines", "20"),
            ],
            ssh("deploy@web1", &["tail", "-n", "20", "--", "/var/log/app.log"]),
        ),
        (
            "files",
            &[("host", "web1"), ("dir", "/srv/app")],
            ssh(
                "web1",
                &[
                    "find",
                    "/srv/app",
                    "-xdev",
                    "-mindepth",
                    "1",
                    "-maxdepth",
                    "2",
                    "-type",
                    "f",
                ],
            ),
        ),
    ];
    for (name, inputs, expected) in cases {
        let (argv, socket) = run(&scopes[name], &program, inputs).unwrap();
        assert_eq!(argv, expected, "{name}");
        assert_eq!(
            socket, agent,
            "{name} uses the agent, and nothing else of Dupin's"
        );
    }

    // Terminating returns no output to the package, so run the profile only to check it binds.
    let profile = exec_profile(&scopes["terminate"], &program).unwrap();
    let argv = profile
        .bind(
            &[
                ("host".to_owned(), exec::Value::Text("web1".into())),
                ("pid".to_owned(), exec::Value::Text("2417".into())),
            ]
            .into(),
        )
        .unwrap();
    assert_eq!(argv, ssh("web1", &["kill", "-s", "TERM", "2417"]));
    assert!(!profile.options().return_output);

    let profile = exec_profile(&scopes["terminal"], &program).unwrap();
    assert!(!profile.options().return_output && !profile.options().ssh_agent);
    let argv = profile
        .bind(
            &[
                ("app".to_owned(), exec::Value::Text("Terminal".into())),
                ("host".to_owned(), exec::Value::Text("web1".into())),
            ]
            .into(),
        )
        .unwrap();
    assert_eq!(argv, ["-a", "Terminal", "ssh://web1"]);
}

#[test]
fn a_host_that_would_read_as_an_ssh_option_never_reaches_ssh() {
    let scopes = scopes();
    let dir = tempfile::tempdir().unwrap();
    let program = fake_ssh(dir.path());
    for (name, inputs) in [
        ("processes", vec![("host", "-oProxyCommand=touch pwned")]),
        ("terminal", vec![("app", "-n"), ("host", "web1")]),
    ] {
        let error = run(&scopes[name], &program, &inputs).unwrap_err();
        assert!(
            matches!(error, exec::ExecError::InvalidInput { .. }),
            "{name}: {error}"
        );
    }
    assert!(!dir.path().join("pwned").exists());
}
