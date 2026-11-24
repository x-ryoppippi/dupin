//! `items()` catalogues, action handlers and the richer rows they share with `search`.

use super::*;
use crate::row::{self, PageChange, RowAction, RowEffect};

fn items(source: &str, cursor: Option<&str>, reason: Reason) -> Result<CataloguePage, RunError> {
    items_with(source, cursor, reason, "{}")
}

fn items_with(
    source: &str,
    cursor: Option<&str>,
    reason: Reason,
    preferences: &str,
) -> Result<CataloguePage, RunError> {
    let (_folder, loaded) = package(&[("main.js", source)]);
    loaded.unwrap().items(
        &engine(),
        cursor,
        reason,
        preferences,
        timeout(DEADLINE),
        &mut Unavailable::default(),
    )
}

fn reset_rows(page: CataloguePage) -> Vec<Row> {
    match page.change {
        PageChange::Reset { rows } => rows,
        change => panic!("expected the whole catalogue, got {change:?}"),
    }
}

fn failure_of<T: std::fmt::Debug>(result: Result<T, RunError>) -> Diagnostic {
    match result {
        Err(RunError::Failed(diagnostic)) => diagnostic,
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn a_package_may_export_items_search_or_both() {
    for (source, exports) in [
        ("export function items() { return []; }", (false, true)),
        (
            "export function items() { return []; }\nexport function search() { return []; }",
            (true, true),
        ),
    ] {
        let (_folder, loaded) = package(&[("main.js", source)]);
        let found = loaded.unwrap().check(&engine(), timeout(DEADLINE)).unwrap();
        assert_eq!((found.search, found.items), exports, "{source}");
    }
    let (_folder, loaded) = package(&[("main.js", "export function search() { return []; }")]);
    assert_eq!(
        failure_of(loaded.unwrap().items(
            &engine(),
            None,
            Reason::Load,
            "{}",
            timeout(DEADLINE),
            &mut Unavailable::default(),
        ))
        .code,
        "missing-items"
    );
}

#[test]
fn items_answers_with_the_whole_catalogue_its_sections_and_details() {
    let page = items(
        r#"export function items(ctx) {
             return {
               ttl: 60,
               cursor: `seen ${ctx.reason} ${ctx.cursor}`,
               sections: [
                 { title: 'Pinned', items: [{ id: 'dupin', title: 'Dupin', keywords: ['launcher'], url: 'https://github.com/x-ryoppippi/dupin',
                   detail: { markdown: '# Dupin\n**fast**', metadata: [{ label: 'Stars', text: '9' }] } }] },
                 { title: 'Other', items: [{ id: 'rune', title: 'Rune', icon: 'assets/rune.png', section: 'Mine',
                   actions: [{ title: 'Forget', run: 'forget', data: { id: 'rune', at: [1, 2] } }] }] },
               ],
             };
           }"#,
        None,
        Reason::Load,
    )
    .unwrap();
    assert_eq!(page.ttl, Some(60));
    assert_eq!(page.cursor.as_deref(), Some("seen load null"));
    assert!(!page.more);
    let PageChange::Reset { rows } = page.change else {
        panic!("{:?}", page.change);
    };
    assert_eq!(rows[0].id, "dupin");
    assert_eq!(rows[0].section.as_deref(), Some("Pinned"));
    assert_eq!(rows[0].keywords, ["launcher"]);
    let detail = rows[0].detail.as_ref().unwrap();
    assert_eq!(detail.markdown, "# Dupin\n**fast**");
    assert_eq!(
        (
            detail.metadata[0].label.as_str(),
            detail.metadata[0].text.as_str()
        ),
        ("Stars", "9")
    );
    assert_eq!(rows[1].id, "rune");
    assert_eq!(rows[1].section.as_deref(), Some("Mine"));
    assert_eq!(rows[1].icon.as_deref(), Some("assets/rune.png"));
    assert_eq!(
        rows[1].actions[0].effect,
        RowEffect::Run {
            handler: "forget".into(),
            data: Some(serde_json::json!({ "id": "rune", "at": [1, 2] })),
        }
    );
    for row in &rows {
        row.validate(row::Source::Items).unwrap();
    }
}

#[test]
fn items_answers_with_changes_by_id_after_its_cursor() {
    let page = items(
        r#"export function items(ctx) {
             return { type: 'update', cursor: '8', more: ctx.reason === 'more',
                      append: [{ id: 'n', title: `after ${ctx.cursor}` }], replace: [{ id: 'o', title: 'Old' }], remove: ['gone'] };
           }"#,
        Some("7"),
        Reason::More,
    )
    .unwrap();
    assert!(page.more);
    assert_eq!(page.cursor.as_deref(), Some("8"));
    page.validate().unwrap();
    let PageChange::Update {
        append,
        replace,
        remove,
        ..
    } = page.change
    else {
        panic!("{:?}", page.change);
    };
    assert_eq!(append[0].title, "after 7");
    assert_eq!(replace[0].id, "o");
    assert_eq!(remove, ["gone"]);
}

#[test]
fn items_problems_name_the_field_at_fault() {
    for (source, code, field) in [
        (
            "export function items() { return { type: 'update', append: [{ title: 'no id' }] }; }",
            "missing-field",
            Some("append[0].id"),
        ),
        (
            "export function items() { return { more: true, items: [{ title: 'no id' }] }; }",
            "missing-field",
            Some("items[0].id"),
        ),
        (
            "export function items() { return [{ title: 'no id' }]; }",
            "missing-field",
            Some("items[0].id"),
        ),
        (
            "export function items() { return { sections: [{ title: 'S', items: [{ title: 'no id' }] }] }; }",
            "missing-field",
            Some("sections[0].items[0].id"),
        ),
        (
            "export function items() { return { type: 'update', append: Object.assign([], { length: 2147483647 }) }; }",
            "too-many-rows",
            None,
        ),
        (
            "export function items() { return { type: 'update', replace: Object.assign([], { length: 2147483647 }) }; }",
            "too-many-rows",
            None,
        ),
        (
            "export function items() { return { type: 'update', append: [{ id: 'a', title: 'a' }], replace: [{ id: 'a', title: 'b' }] }; }",
            "duplicate-id",
            Some("replace[0].id"),
        ),
        (
            "export function items() { return { type: 'update', remove: [1] }; }",
            "invalid-field",
            Some("remove[0]"),
        ),
        (
            "export function items() { return { ttl: 0, items: [] }; }",
            "invalid-field",
            Some("ttl"),
        ),
        (
            "export function items() { return { ttl: 1.5, items: [] }; }",
            "invalid-field",
            Some("ttl"),
        ),
        (
            "export function items() { return { more: 'yes', items: [] }; }",
            "invalid-field",
            Some("more"),
        ),
        (
            "export function items() { return { type: 'update', items: [] }; }",
            "unknown-field",
            Some("items"),
        ),
        (
            "export function items() { return { type: 'detail' }; }",
            "invalid-field",
            Some("type"),
        ),
        (
            "export function items() { return { cursor: 'x'.repeat(257), items: [] }; }",
            "field-too-long",
            Some("cursor"),
        ),
        (
            "export function items() { return [{ id: 'a', title: 'a', keywords: Array(17).fill('k') }]; }",
            "invalid-field",
            Some("items[0].keywords"),
        ),
        (
            "export function items() { return Array.from({ length: 501 }, (_, i) => ({ id: `${i}`, title: 'a' })); }",
            "too-many-rows",
            None,
        ),
        (
            "export function items() { return Array.from({ length: 300 }, (_, i) => ({ id: `${i}`, title: 'a', subtitle: 'x'.repeat(2048), text: 'y'.repeat(2048) })); }",
            "output-too-large",
            Some("items[255].text"),
        ),
        (
            "export function items() { throw new Error('nope'); }",
            "items-threw",
            None,
        ),
    ] {
        let diagnostic = failure_of(items(source, None, Reason::Load));
        assert_eq!(
            (diagnostic.code.as_ref(), diagnostic.field.as_deref()),
            (code, field),
            "{source}: {diagnostic:?}"
        );
        assert!(diagnostic.hint.is_some(), "{source}: {diagnostic:?}");
    }
}

#[test]
fn an_action_handler_runs_with_its_data_and_may_invalidate_the_catalogue() {
    let (_folder, loaded) = package(&[(
        "main.js",
        r#"import { items as catalogue } from 'dupin';
           export function items() { return []; }
           export function search() { catalogue.invalidate(); return []; }
           export const actions = {
             forget(data) { if (data.id === 'rune') catalogue.invalidate(); },
             async later() { await null; },
             answer() { return 1; },
           };"#,
    )]);
    let package = loaded.unwrap();
    let act = |name: &str, data: &str| {
        package.action(
            &engine(),
            name,
            data,
            "{}",
            timeout(DEADLINE),
            &mut Unavailable::default(),
        )
    };
    assert_eq!(
        act("forget", r#"{"id":"rune"}"#).unwrap(),
        Acted {
            invalidated: true,
            effect: None
        }
    );
    assert_eq!(act("forget", r#"{"id":"dupin"}"#).unwrap(), Acted::default());
    assert_eq!(act("later", "null").unwrap(), Acted::default());
    let returned = failure_of(act("answer", "null"));
    assert_eq!(returned.code, "invalid-effect");
    let missing = failure_of(act("absent", "null"));
    assert_eq!(
        (missing.code.as_ref(), missing.field.as_deref()),
        ("missing-handler", Some("actions.absent"))
    );
    let outside = failure_of(package.search("", 5, timeout(DEADLINE)));
    assert_eq!(outside.code, "search-threw");
    assert!(
        outside.message.contains("only in an action or event handler"),
        "{outside:?}"
    );
}

#[test]
fn the_host_hook_is_gone_before_package_code_runs() {
    let rows = run(
        r#"import { apiVersion } from 'dupin';
           export function search() {
             return [{ title: `${apiVersion} ${typeof globalThis.__dupinHost} ${typeof globalThis.__dupinInvalidate}` }];
           }"#,
        "",
    )
    .unwrap();
    assert_eq!(rows[0].title, "2 undefined undefined");
}

#[test]
fn search_rows_may_carry_sections_details_icons_and_actions() {
    let rows = run(
        r#"export function search(query) {
             return { sections: [{ title: 'Answers', items: [{ title: query, icon: 'icon.svg',
               detail: { markdown: '`code`' }, actions: [{ title: 'Open', open: 'https://example.com/' }, { title: 'Again', run: 'again' }] }] }] };
           }"#,
        "two",
    )
    .unwrap();
    assert_eq!(rows[0].section.as_deref(), Some("Answers"));
    assert_eq!(rows[0].icon.as_deref(), Some("icon.svg"));
    assert_eq!(rows[0].detail.as_ref().unwrap().markdown, "`code`");
    assert_eq!(rows[0].actions.len(), 2);
    rows[0].validate(row::Source::Search).unwrap();
}

#[test]
fn rows_and_details_name_images_by_handle_only() {
    let handle = "a".repeat(64);
    let rows = run(
        &format!(
            r#"export function search() {{
                 return [{{ title: 'Stored', image: '{handle}', detail: {{ image: '{handle}' }} }},
                         {{ title: 'Copied', image: {{ path: '/Users/Shared/a.png', handle: '{handle}' }} }}];
               }}"#
        ),
        "x",
    )
    .unwrap();
    assert_eq!(rows[0].image, Some(row::RowImage::Blob(handle.clone())));
    assert_eq!(
        rows[0].detail.as_ref().unwrap().image,
        Some(row::RowImage::Blob(handle.clone()))
    );
    assert_eq!(
        rows[1].image,
        Some(row::RowImage::File(row::CopiedFileImage {
            path: "/Users/Shared/a.png".into(),
            handle: handle.clone(),
        }))
    );
    for shown in &rows {
        shown.validate(row::Source::Search).unwrap();
    }

    for (image, field) in [
        ("42", "items[0].image"),
        ("'not a handle'", "items[0].image"),
        ("''", "items[0].image"),
        ("{ path: 'a.png', handle: 'h' }", "items[0].image"),
        ("{ path: '/a.png' }", "items[0].image"),
        (
            "{ path: '/a.png', handle: 'h', bytes: 'AA==' }",
            "items[0].image.bytes",
        ),
    ] {
        let diagnostic = match run(
            &format!("export function search() {{ return [{{ title: 'a', image: {image} }}]; }}"),
            "x",
        ) {
            Err(RunError::Failed(diagnostic)) => diagnostic,
            other => panic!("{image}: {other:?}"),
        };
        assert_eq!(diagnostic.field.as_deref(), Some(field), "{image}");
    }
}

#[test]
fn rows_from_another_process_are_checked_again() {
    let row = |edit: &dyn Fn(&mut Row)| {
        let mut row = Row {
            id: "a".into(),
            title: "A".into(),
            subtitle: String::new(),
            keywords: Vec::new(),
            match_text: None,
            time: None,
            file: None,
            icon: None,
            image: None,
            section: None,
            detail: None,
            actions: Vec::new(),
            web_search: None,
        };
        edit(&mut row);
        row
    };
    assert!(row(&|_| {}).validate(row::Source::Search).is_ok());
    let action = |effect: RowEffect| RowAction::new("Act", effect);
    for broken in [
        row(&|row| row.title = "a\nb".into()),
        row(&|row| row.keywords = vec!["k".into()]),
        row(&|row| row.icon = Some("/etc/hosts.png".into())),
        row(&|row| row.icon = Some("a/../../b.png".into())),
        row(&|row| row.icon = Some("icon.exe".into())),
        row(&|row| row.image = Some(crate::row::RowImage::Blob("a b".into()))),
        row(&|row| {
            row.image = Some(crate::row::RowImage::File(crate::row::CopiedFileImage {
                path: "relative.png".into(),
                handle: "h".into(),
            }))
        }),
        row(&|row| {
            row.actions = vec![action(RowEffect::OpenUrl {
                url: "file:///".into(),
            })]
        }),
        row(&|row| {
            row.actions = vec![action(RowEffect::Run {
                handler: "a b".into(),
                data: None,
            })]
        }),
    ] {
        assert!(broken.validate(row::Source::Search).is_err(), "{broken:?}");
    }
    assert!(
        row(&|row| row.keywords = vec!["k".into()])
            .validate(row::Source::Items)
            .is_ok()
    );
}

#[test]
fn items_sees_the_preferences_frozen_through_and_through() {
    let preferences = r#"{ "webSearches": { "docs": { "name": "Docs", "url": "https://docs.example/?q={query}" } }, "webFallbacks": ["docs"] }"#;
    let source = r#"export function items(ctx) {
         const searches = ctx.preferences.webSearches ?? {};
         let frozen = Object.isFrozen(ctx.preferences) && Object.isFrozen(searches.docs ?? {})
           && Object.isFrozen(ctx.preferences.webFallbacks ?? []);
         try { ctx.preferences.webSearches.docs.name = 'Changed'; } catch { }
         return Object.entries(searches).map(([alias, { name }]) => ({
           id: alias, title: `${name} ${frozen} ${ctx.preferences.webFallbacks[0]}`,
         }));
       }"#;
    let rows = reset_rows(items_with(source, None, Reason::Load, preferences).unwrap());
    assert_eq!(rows[0].title, "Docs true docs");
    let empty = r#"export function items(ctx) { return [{ id: 'a', title: String(Object.keys(ctx.preferences).length) }]; }"#;
    assert_eq!(
        reset_rows(items(empty, None, Reason::Load).unwrap())[0].title,
        "0"
    );
}

#[test]
fn a_catalogue_item_may_be_a_web_search_that_dupin_fills() {
    let rows = reset_rows(
        items(
            r#"export function items() {
                 return [
                   { id: 'g', title: 'Search Google', subtitle: 'g',
                     webSearch: { alias: 'g', url: 'https://www.google.com/search?q={query}', title: 'Search Google for {query}', fallback: 2 } },
                   { id: 'maps', title: 'Search Apple Maps',
                     webSearch: { alias: 'maps', url: 'maps://?q={query}', title: 'Search Apple Maps for {query}' } },
                 ];
               }"#,
            None,
            Reason::Load,
        )
        .unwrap(),
    );
    let google = rows[0].web_search.as_ref().unwrap();
    assert_eq!((google.alias.as_str(), google.fallback), ("g", Some(2)));
    assert!(rows[0].actions.is_empty());
    assert!(rows[1].web_search.as_ref().unwrap().opens_maps());
    rows[0].validate(row::Source::Items).unwrap();
    assert!(rows[0].validate(row::Source::Search).is_err());

    for (search, field) in [
        (
            "{ alias: 'g g', url: 'https://e.example/?q={query}', title: '{query}' }",
            "items[0].webSearch.alias",
        ),
        (
            "{ alias: 'g', url: 'file:///{query}', title: '{query}' }",
            "items[0].webSearch.url",
        ),
        (
            "{ alias: 'g', url: 'https://e.example/?q={query}', title: 'Search' }",
            "items[0].webSearch.title",
        ),
        (
            "{ alias: 'g', url: 'https://e.example/?q={query}', title: '{query}', fallback: 1.5 }",
            "items[0].webSearch.fallback",
        ),
        (
            "{ alias: 'g', url: 'https://e.example/?q={query}' }",
            "items[0].webSearch.title",
        ),
        (
            "{ alias: 'g', url: 'https://e.example/?q={query}', title: '{query}', open: true }",
            "items[0].webSearch.open",
        ),
        ("'g'", "items[0].webSearch"),
    ] {
        let source =
            format!("export function items() {{ return [{{ id: 'g', title: 'G', webSearch: {search} }}]; }}");
        let diagnostic = failure_of(items(&source, None, Reason::Load));
        assert_eq!(
            diagnostic.field.as_deref(),
            Some(field),
            "{search}: {diagnostic:?}"
        );
    }
    let with_url = "export function items() { return [{ id: 'g', title: 'G', url: 'https://e.example/', webSearch: { alias: 'g', url: 'https://e.example/?q={query}', title: '{query}' } }]; }";
    assert_eq!(
        failure_of(items(with_url, None, Reason::Load)).field.as_deref(),
        Some("items[0].webSearch")
    );
    let (_folder, loaded) = package(&[(
        "main.js",
        "export function search() { return [{ title: 'G', webSearch: { alias: 'g', url: 'https://e.example/?q={query}', title: '{query}' } }]; }",
    )]);
    let diagnostic = failure_of(loaded.unwrap().search("x", 5, timeout(DEADLINE)));
    assert_eq!(diagnostic.code, "unknown-field");
}
