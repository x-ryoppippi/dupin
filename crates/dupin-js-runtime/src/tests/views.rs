use serde_json::json;

use super::*;
use crate::output::ActionEffect;
use crate::row::{Accessory, RowEffect};
use crate::view::{FieldKind, ViewAnswer};

const VIEWS: &str = r#"
export const search = (query) => [
    { title: 'Note', accessories: [{ text: '2 KB' }, { tag: 'New' }], answer: '42',
      actions: [{ title: 'Open', push: { view: 'note', state: { id: 7 } } }] },
];
export const views = {
    note(ctx) {
        return { type: 'detail', title: `Note ${ctx.state.id}`, markdown: ctx.query,
                 actions: [{ title: 'Edit', push: { view: 'edit', state: ctx.state } }] };
    },
    list(ctx) { return { type: 'list', items: [], emptyText: `Nothing for ${ctx.query}` }; },
    edit(ctx) {
        return { type: 'form', title: 'Edit', submit: 'save', submitTitle: 'Save', fields: [
            { type: 'text', id: 'title', title: 'Title', required: true, placeholder: 'Untitled' },
            { type: 'textarea', id: 'body', title: 'Body', default: 'a\nb' },
            { type: 'checkbox', id: 'pinned', title: 'Pinned', default: true },
            { type: 'select', id: 'label', title: 'Label', options: [
                { value: 'red', title: 'Red' }, { value: 'blue', title: 'Blue' }] },
        ] };
    },
    frozen(ctx) {
        try { ctx.state.id = 9; } catch {}
        return [{ title: `${Object.isFrozen(ctx)} ${Object.isFrozen(ctx.state)} ${ctx.state.id} ${typeof ctx.preferences}` }];
    },
    counter() { globalThis.seen = (globalThis.seen ?? 0) + 1; return [{ title: String(globalThis.seen) }]; },
    mutate() { items.invalidate(); return []; },
};
import { items } from 'dupin';
"#;

fn view(source: &str, name: &str, query: &str, state: serde_json::Value) -> Result<ViewAnswer, RunError> {
    let (_folder, loaded) = package(&[("main.js", source)]);
    loaded.unwrap().view(
        &engine(),
        name,
        &PageContext {
            query: query.into(),
            state,
        },
        &json!({}),
        timeout(DEADLINE),
        &mut Unavailable::default(),
    )
}

fn view_failure(source: &str, name: &str) -> Diagnostic {
    match view(source, name, "", serde_json::Value::Null) {
        Err(RunError::Failed(diagnostic)) => diagnostic,
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn rows_carry_accessories_answers_and_push_actions() {
    let rows = run(VIEWS, "x").unwrap();
    assert_eq!(
        rows[0].accessories,
        [Accessory::Text("2 KB".into()), Accessory::Tag("New".into())]
    );
    assert_eq!(rows[0].answer.as_deref(), Some("42"));
    assert_eq!(
        rows[0].actions[0].effect,
        RowEffect::Push {
            view: "note".into(),
            state: Some(json!({"id": 7})),
        }
    );
    rows[0].validate(crate::row::Source::Search).unwrap();
}

#[test]
fn views_answer_a_detail_a_list_or_a_form_for_the_page() {
    let ViewAnswer::Detail {
        title,
        detail,
        actions,
    } = view(VIEWS, "note", "typed", json!({"id": 7})).unwrap()
    else {
        panic!()
    };
    assert_eq!((title.as_str(), detail.markdown.as_str()), ("Note 7", "typed"));
    assert_eq!(actions[0].title, "Edit");

    assert_eq!(
        view(VIEWS, "list", "tea", serde_json::Value::Null).unwrap(),
        ViewAnswer::List {
            rows: Vec::new(),
            empty_text: Some("Nothing for tea".into()),
        }
    );

    let answer = view(VIEWS, "edit", "", json!({"id": 7})).unwrap();
    answer.validate().unwrap();
    let ViewAnswer::Form(form) = answer else { panic!() };
    assert_eq!(
        (form.submit.as_str(), form.submit_title.as_deref()),
        ("save", Some("Save"))
    );
    assert!(matches!(
        &form.fields[0].kind,
        FieldKind::Text { required: true, placeholder: Some(placeholder), .. } if placeholder == "Untitled"
    ));
    assert!(matches!(&form.fields[1].kind, FieldKind::Textarea { default, .. } if default == "a\nb"));
    assert!(matches!(
        form.fields[2].kind,
        FieldKind::Checkbox { default: true }
    ));
    assert!(matches!(&form.fields[3].kind, FieldKind::Select { default, .. } if default == "red"));
}

#[test]
fn a_view_gets_a_frozen_context_and_fresh_module_state_each_call() {
    let ViewAnswer::List { rows, .. } = view(VIEWS, "frozen", "", json!({"id": 7})).unwrap() else {
        panic!()
    };
    assert_eq!(rows[0].title, "true true 7 object");
    for _ in 0..2 {
        let ViewAnswer::List { rows, .. } = view(VIEWS, "counter", "", serde_json::Value::Null).unwrap()
        else {
            panic!()
        };
        assert_eq!(rows[0].title, "1", "module globals never outlive a call");
    }
}

#[test]
fn a_view_may_not_invalidate_the_catalogue_or_name_what_it_does_not_export() {
    assert_eq!(view_failure(VIEWS, "mutate").code, "view-threw");
    let missing = view_failure(VIEWS, "toString");
    assert_eq!(
        (missing.code.as_ref(), missing.field.as_deref()),
        ("missing-handler", Some("views.toString"))
    );
    assert_eq!(view_failure(VIEWS, "absent").code, "missing-handler");
    assert_eq!(
        view_failure("export const search = () => [];", "note").code,
        "missing-handler"
    );
}

#[test]
fn malformed_views_name_the_field_at_fault() {
    for (source, code, field) in [
        ("{ type: 'grid' }", "invalid-field", "type"),
        ("{ type: 'detail' }", "missing-field", "title"),
        (
            "{ type: 'detail', title: 'x', extra: 1 }",
            "unknown-field",
            "extra",
        ),
        (
            "{ type: 'list', items: [], emptyText: 'a\\nb' }",
            "invalid-field",
            "emptyText",
        ),
        (
            "{ type: 'form', title: 'x', submit: 'save', fields: [] }",
            "invalid-field",
            "fields",
        ),
        (
            "{ type: 'form', title: 'x', submit: '1bad', fields: [{ type: 'text', id: 'a', title: 'A' }] }",
            "invalid-field",
            "submit",
        ),
        (
            "{ type: 'form', title: 'x', submit: 's', fields: [{ type: 'date', id: 'a', title: 'A' }] }",
            "invalid-field",
            "fields[0].type",
        ),
        (
            "{ type: 'form', title: 'x', submit: 's', fields: [{ type: 'text', id: 'a b', title: 'A' }] }",
            "invalid-field",
            "fields[0].id",
        ),
        (
            "{ type: 'form', title: 'x', submit: 's', fields: [{ type: 'text', id: 'a', title: 'A' }, { type: 'checkbox', id: 'a', title: 'B' }] }",
            "invalid-form",
            "fields",
        ),
        (
            "{ type: 'form', title: 'x', submit: 's', fields: [{ type: 'text', id: 'a', title: 'A', default: 'x\\ny' }] }",
            "invalid-field",
            "fields[0].default",
        ),
        (
            "{ type: 'form', title: 'x', submit: 's', fields: [{ type: 'select', id: 'a', title: 'A', options: [{ value: 'x', title: 'X' }], default: 'y' }] }",
            "invalid-form",
            "fields",
        ),
        (
            "{ type: 'form', title: 'x', submit: 's', fields: [{ type: 'checkbox', id: 'a', title: 'A', required: true }] }",
            "unknown-field",
            "fields[0].required",
        ),
        (
            "{ type: 'form', title: 'x', submit: 's', fields: [{ type: 'textarea', id: 'a', title: 'A', default: 'x'.repeat(16 * 1024 + 1) }] }",
            "field-too-long",
            "fields[0].default",
        ),
        (
            "{ type: 'form', title: 'x', submit: 's', fields: Array.from({ length: 17 }, (_, i) => ({ type: 'checkbox', id: `f${i}`, title: 'A' })) }",
            "invalid-field",
            "fields",
        ),
    ] {
        let source =
            format!("export const search = () => []; export const views = {{ v: () => ({source}) }};");
        let diagnostic = view_failure(&source, "v");
        assert_eq!(
            (diagnostic.code.as_ref(), diagnostic.field.as_deref()),
            (code, Some(field)),
            "{source}: {diagnostic:?}"
        );
    }
}

#[test]
fn a_row_from_another_process_answers_in_one_or_two_lines_within_the_limit() {
    let answered = |answer: String| {
        let mut row = crate::row::Row::new("a", "a");
        row.answer = Some(answer);
        row.validate(crate::row::Source::Search)
    };
    assert!(answered("≈ £42.50".into()).is_ok());
    assert!(answered("≈ £42.50\nECB rate 2026-11-20".into()).is_ok());
    for refused in [
        String::new(),
        "a\n".into(),
        "\nb".into(),
        "a\nb\nc".into(),
        format!("{}\nb", "a".repeat(511)),
    ] {
        assert!(answered(refused.clone()).is_err(), "{refused:?}");
    }
}

#[test]
fn malformed_accessories_answers_and_pushes_are_refused() {
    for (item, code, field) in [
        (
            "{ title: 'a', accessories: [{ text: 'a' }, { text: 'b' }, { text: 'c' }, { text: 'd' }] }",
            "invalid-field",
            "items[0].accessories",
        ),
        (
            "{ title: 'a', accessories: [{ text: 'a', tag: 'b' }] }",
            "invalid-field",
            "items[0].accessories[0]",
        ),
        (
            "{ title: 'a', accessories: [{ text: 'x'.repeat(65) }] }",
            "field-too-long",
            "items[0].accessories[0].text",
        ),
        (
            "{ title: 'a', answer: 'a\\nb\\nc' }",
            "invalid-field",
            "items[0].answer",
        ),
        (
            "{ title: 'a', answer: 'a\\n' }",
            "invalid-field",
            "items[0].answer",
        ),
        (
            "{ title: 'a', actions: [{ title: 'Go', push: 'note' }] }",
            "invalid-field",
            "items[0].actions[0].push",
        ),
        (
            "{ title: 'a', actions: [{ title: 'Go', push: { view: 'no such' } }] }",
            "invalid-field",
            "items[0].actions[0].push.view",
        ),
        (
            "{ title: 'a', actions: [{ title: 'Go', push: { view: 'v', state: 'x'.repeat(16 * 1024) } }] }",
            "invalid-field",
            "items[0].actions[0].push.state",
        ),
        (
            "{ title: 'a', actions: [{ title: 'Go', push: { view: 'v', state: () => 1 } }] }",
            "invalid-field",
            "items[0].actions[0].push.state",
        ),
        (
            "{ title: 'a', actions: [{ title: 'Go', push: { view: 'v' }, data: 1 }] }",
            "unknown-field",
            "items[0].actions[0].data",
        ),
    ] {
        let diagnostic = failure(run(&format!("export const search = () => [{item}];"), "x"));
        assert_eq!(
            (diagnostic.code.as_ref(), diagnostic.field.as_deref()),
            (code, Some(field)),
            "{item}: {diagnostic:?}"
        );
    }
    let nested = failure(run(
        "export const search = () => { let state = 1; for (let i = 0; i < 20; i++) state = [state]; return [{ title: 'a', actions: [{ title: 'Go', push: { view: 'v', state } }] }]; };",
        "x",
    ));
    assert_eq!(nested.field.as_deref(), Some("items[0].actions[0].push.state"));
}

#[test]
fn a_catalogue_takes_accessories_but_not_answers() {
    let (_folder, loaded) = package(&[(
        "main.js",
        "export const items = () => [{ id: 'a', title: 'A', answer: '1' }];",
    )]);
    let diagnostic = match loaded.unwrap().items(
        &engine(),
        None,
        Reason::Load,
        &json!({}),
        timeout(DEADLINE),
        &mut Unavailable::default(),
    ) {
        Err(RunError::Failed(diagnostic)) => diagnostic,
        other => panic!("{other:?}"),
    };
    assert_eq!(diagnostic.field.as_deref(), Some("items[0].answer"));
}

fn effect(answer: &str) -> Result<Option<ActionEffect>, RunError> {
    let (_folder, loaded) = package(&[(
        "main.js",
        &format!(
            "export const search = () => []; export const actions = {{ go: (data, ctx) => ({answer}) }};"
        ),
    )]);
    loaded
        .unwrap()
        .action(
            &engine(),
            "go",
            "null",
            &PageContext {
                query: "typed".into(),
                state: json!({"n": 1}),
            },
            &json!({}),
            timeout(DEADLINE),
            &mut Unavailable::default(),
        )
        .map(|acted| acted.effect)
}

#[test]
fn actions_see_their_page_and_return_navigation_effects() {
    assert_eq!(
        effect("({ type: 'toast', text: `${ctx.query} ${ctx.state.n}` })").unwrap(),
        Some(ActionEffect::Toast {
            text: "typed 1".into()
        })
    );
    assert_eq!(
        effect(
            "({ type: 'push', view: 'note', title: 'Note', state: { n: ctx.state.n + 1 }, toast: 'Opened' })"
        )
        .unwrap(),
        Some(ActionEffect::Push {
            view: "note".into(),
            title: Some("Note".into()),
            state: Some(json!({"n": 2})),
            toast: Some("Opened".into()),
        })
    );
    assert_eq!(
        effect("({ type: 'pop', toast: 'Saved' })").unwrap(),
        Some(ActionEffect::Pop {
            toast: Some("Saved".into())
        })
    );
    assert_eq!(
        effect("({ type: 'refresh' })").unwrap(),
        Some(ActionEffect::Refresh { toast: None })
    );
    assert_eq!(
        effect("({ type: 'replaceState', state: null })").unwrap(),
        Some(ActionEffect::ReplaceState {
            state: serde_json::Value::Null,
            toast: None
        })
    );
    assert_eq!(
        effect("({ type: 'fieldErrors', errors: { title: 'Taken' } })").unwrap(),
        Some(ActionEffect::FieldErrors {
            errors: [("title".to_owned(), "Taken".to_owned())].into()
        })
    );
    for effect in [
        ActionEffect::Push {
            view: "note".into(),
            title: None,
            state: None,
            toast: None,
        },
        ActionEffect::FieldErrors {
            errors: [("title".to_owned(), "Taken".to_owned())].into(),
        },
    ] {
        effect.validate().unwrap();
    }
}

#[test]
fn malformed_effects_are_refused_before_dupin_acts_on_them() {
    for (answer, field) in [
        ("({ type: 'push' })", "view"),
        ("({ type: 'push', view: 'a b' })", "view"),
        (
            "({ type: 'push', view: 'v', state: 'x'.repeat(16 * 1024) })",
            "state",
        ),
        ("({ type: 'push', view: 'v', state: () => 1 })", "state"),
        ("({ type: 'push', view: 'v', toast: 'a\\nb' })", "toast"),
        ("({ type: 'pop', text: 'x' })", "text"),
        ("({ type: 'replaceState' })", "state"),
        ("({ type: 'replaceState', state: 1, view: 'v' })", "view"),
        ("({ type: 'fieldErrors', errors: [] })", "errors"),
        ("({ type: 'fieldErrors', errors: {} })", "errors"),
        ("({ type: 'fieldErrors', errors: { 'a b': 'x' } })", "errors"),
        (
            "({ type: 'fieldErrors', errors: { a: 'x'.repeat(257) } })",
            "errors",
        ),
        ("({ type: 'fieldErrors', errors: { a: 1 } })", "errors"),
        (
            "({ type: 'fieldErrors', errors: Object.fromEntries(Array.from({ length: 17 }, (_, i) => [`f${i}`, 'x'])) })",
            "errors",
        ),
        ("({ type: 'navigate', view: 'v' })", "type"),
    ] {
        let diagnostic = match effect(answer) {
            Err(RunError::Failed(diagnostic)) => diagnostic,
            other => panic!("{answer}: {other:?}"),
        };
        assert_eq!(
            (diagnostic.code.as_ref(), diagnostic.field.as_deref()),
            ("invalid-effect", Some(field)),
            "{answer}: {diagnostic:?}"
        );
    }
    let oversized = ActionEffect::ReplaceState {
        state: json!("x".repeat(16 * 1024)),
        toast: None,
    };
    assert!(
        oversized.validate().is_err(),
        "the host checks a helper's effects again"
    );
    assert!(
        ActionEffect::Pop {
            toast: Some(String::new())
        }
        .validate()
        .is_err()
    );
}

#[test]
fn pages_from_another_process_are_checked_again() {
    let form = |edit: &dyn Fn(&mut crate::view::Form)| {
        let ViewAnswer::Form(mut form) = view(VIEWS, "edit", "", serde_json::Value::Null).unwrap() else {
            panic!()
        };
        edit(&mut form);
        ViewAnswer::Form(form).validate()
    };
    assert!(form(&|_| {}).is_ok());
    assert!(form(&|form| form.fields.clear()).is_err());
    assert!(form(&|form| form.submit = "a b".into()).is_err());
    assert!(form(&|form| form.fields[1].id = "title".into()).is_err());
    assert!(form(&|form| form.fields[0].title = "a\nb".into()).is_err());
    assert!(
        form(
            &|form| if let FieldKind::Select { default, .. } = &mut form.fields[3].kind {
                *default = "green".into();
            }
        )
        .is_err()
    );
    let mut rows = vec![Row::new("a", "A"), Row::new("a", "B")];
    assert!(
        ViewAnswer::List {
            rows: rows.clone(),
            empty_text: None
        }
        .validate()
        .is_err()
    );
    rows.truncate(1);
    rows[0].accessories = vec![Accessory::Tag("x".repeat(65))];
    assert!(
        ViewAnswer::List {
            rows,
            empty_text: None
        }
        .validate()
        .is_err()
    );
}
