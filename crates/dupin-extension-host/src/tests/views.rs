use dupin_js_runtime::row::{Accessory, RowEffect};
use dupin_js_runtime::{ActionEffect, PageContext, ViewAnswer};
use serde_json::json;

use super::*;

const VIEWS: &str = r#"
export const search = () => [{ title: 'Note', actions: [{ title: 'Open', push: { view: 'note', state: { id: 7 } } }] }];
export const views = {
    note: (ctx) => [{ title: `${ctx.query} ${ctx.state.id}`, accessories: [{ tag: 'New' }] }],
    spin() { for (;;) {} },
};
export const actions = {
    deeper: (data, ctx) => ({ type: 'push', view: 'note', state: { id: ctx.state.id + 1 }, toast: ctx.query }),
};
"#;

fn page(query: &str, state: serde_json::Value) -> PageContext {
    PageContext {
        query: query.into(),
        state,
    }
}

#[test]
fn views_run_in_the_resident_helper_and_actions_see_the_page_they_were_chosen_on() {
    skip_inside_a_sandbox!();
    let host = host();
    let package = package(VIEWS);
    let deadline = || Instant::now() + Duration::from_secs(5);
    let rows = search(&host, &package, "", Duration::from_secs(5)).unwrap();
    assert_eq!(
        rows[0].actions[0].effect,
        RowEffect::Push {
            view: "note".into(),
            state: Some(json!({"id": 7})),
        }
    );
    let resident = host.helper_pid(&package);
    let view = host
        .view(
            &package,
            "note",
            &page("tea", json!({"id": 7})),
            &json!({}),
            deadline(),
            &|| false,
            &none(),
        )
        .unwrap();
    let ViewAnswer::List { rows, .. } = view else {
        panic!("{view:?}")
    };
    assert_eq!(rows[0].title, "tea 7");
    assert_eq!(rows[0].accessories, [Accessory::Tag("New".into())]);
    assert_eq!(
        host.helper_pid(&package),
        resident,
        "a view runs as a search does"
    );

    let acted = host
        .action(
            &package,
            "deeper",
            &serde_json::Value::Null,
            &page("tea", json!({"id": 7})),
            &json!({}),
            deadline(),
            &|| false,
            &none(),
        )
        .unwrap();
    assert_eq!(
        acted.effect,
        Some(ActionEffect::Push {
            view: "note".into(),
            title: None,
            state: Some(json!({"id": 8})),
            toast: Some("tea".into()),
        })
    );
}

#[test]
fn a_view_that_never_returns_ends_by_its_deadline_and_the_next_one_answers() {
    skip_inside_a_sandbox!();
    let host = host();
    let package = package(VIEWS);
    let started = Instant::now();
    let stuck = failed(host.view(
        &package,
        "spin",
        &PageContext::default(),
        &json!({}),
        Instant::now() + Duration::from_millis(300),
        &|| false,
        &none(),
    ));
    assert_eq!(stuck.code, "deadline", "{stuck:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        host.view(
            &package,
            "note",
            &page("", json!({"id": 1})),
            &json!({}),
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &none(),
        )
        .is_ok()
    );
}

const FORGING_ENV: &str = "DUPIN_EXTENSION_HOST_FORGING";

/// Not a test: a subverted helper that answers each view with a page Dupin may not show and
/// each action with an effect Dupin may not carry out.
#[test]
fn forging_helper_entry() {
    use std::os::fd::FromRawFd;
    use std::os::unix::net::UnixStream;

    use wire::{Answer, Call, FromHelper, ToHelper};

    if std::env::var_os(FORGING_ENV).is_none() {
        return;
    }
    // SAFETY: the host starts this process with the socket as standard input.
    let mut socket = unsafe { UnixStream::from_raw_fd(0) };
    let hello = FromHelper::Hello {
        version: wire::VERSION,
        engine: dupin_js_runtime::engine_version().into(),
    };
    wire::write(&mut socket, &hello, &[]).unwrap();
    while let Ok(Some((message, _))) = wire::read::<ToHelper>(&mut socket) {
        let ToHelper::Run { id, call, .. } = message else {
            continue;
        };
        let answer = match call {
            Call::View { name, .. } if name == "twins" => Answer::View {
                view: ViewAnswer::List {
                    rows: vec![Row::new("a", "A"), Row::new("a", "B")],
                    empty_text: None,
                },
            },
            Call::View { .. } => Answer::View {
                view: ViewAnswer::List {
                    rows: vec![Row {
                        actions: vec![dupin_js_runtime::row::RowAction::new(
                            "Open",
                            RowEffect::Push {
                                view: "note".into(),
                                state: Some(json!("x".repeat(64 * 1024))),
                            },
                        )],
                        ..Row::new("a", "A")
                    }],
                    empty_text: None,
                },
            },
            Call::Action { name, .. } if name == "errors" => Answer::Acted {
                acted: dupin_js_runtime::Acted {
                    invalidated: false,
                    effect: Some(ActionEffect::FieldErrors {
                        errors: [("a b".to_owned(), "x".to_owned())].into(),
                    }),
                },
            },
            Call::Action { .. } => Answer::Acted {
                acted: dupin_js_runtime::Acted {
                    invalidated: false,
                    effect: Some(ActionEffect::ReplaceState {
                        state: json!("x".repeat(64 * 1024)),
                        toast: None,
                    }),
                },
            },
            _ => Answer::Failed {
                diagnostic: Diagnostic::new("code", "message"),
            },
        };
        if wire::write(
            &mut socket,
            &FromHelper::Done {
                id,
                answer,
                answered: Vec::new(),
            },
            &[],
        )
        .is_err()
        {
            break;
        }
    }
    std::process::exit(0);
}

#[test]
fn pages_and_effects_a_subverted_helper_forges_are_refused() {
    skip_inside_a_sandbox!();
    let package = package(VIEWS);
    let host = Host::new(Box::new(|| {
        let mut command = helper_command("tests::views::forging_helper_entry")?;
        command.env(FORGING_ENV, "1");
        Ok(command)
    }));
    let deadline = || Instant::now() + Duration::from_secs(5);
    for name in ["oversized", "twins"] {
        let refused = failed(host.view(
            &package,
            name,
            &PageContext::default(),
            &json!({}),
            deadline(),
            &|| false,
            &none(),
        ));
        assert_eq!(refused.code, "host-error", "{name}: {refused:?}");
    }
    for name in ["oversized", "errors"] {
        let refused = failed(host.action(
            &package,
            name,
            &serde_json::Value::Null,
            &PageContext::default(),
            &json!({}),
            deadline(),
            &|| false,
            &none(),
        ));
        assert_eq!(refused.code, "host-error", "{name}: {refused:?}");
        assert!(refused.message.contains("effect"), "{refused:?}");
    }
}
