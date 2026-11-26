//! Edits config.toml in place: a save rewrites only the keys whose value changed, so the
//! comments, order and layout the user gave the rest of the file stay as they wrote them.

use dupin_core::Config;
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, RawString, Table, TableLike, Value};

/// `source` with the keys that differ between `before`, which `source` reads as, and `after`
/// set to their new values, and the keys `after` leaves out removed.
pub(super) fn edit(source: &str, before: &Config, after: &Config) -> Result<String, String> {
    const ENCODE: &str = "could not encode config.toml";
    let mut document: DocumentMut = source.parse().map_err(|_| "could not edit config.toml")?;
    let values = |config: &Config| toml::Table::try_from(config).map_err(|_| ENCODE);
    // `toml::Table` sorts its keys; this keeps a new key's fields in the order Config declares.
    let written: DocumentMut = toml::to_string(after)
        .ok()
        .and_then(|source| source.parse().ok())
        .ok_or(ENCODE)?;
    apply(
        document.as_table_mut(),
        true,
        &values(before)?,
        &values(after)?,
        written.as_table(),
    );
    let edited = document.to_string();
    if Config::parse(&edited).as_ref() != Ok(after) {
        return Err(ENCODE.into());
    }
    Ok(edited)
}

/// `standard` says whether `table` is written with its own header, so a table added to it can
/// take one too rather than being written inline. `written` is `after` as TOML.
fn apply(
    table: &mut dyn TableLike,
    standard: bool,
    before: &toml::Table,
    after: &toml::Table,
    written: &dyn TableLike,
) {
    for key in before.keys().filter(|key| !after.contains_key(*key)) {
        table.remove(key);
    }
    for (key, written) in written.iter() {
        let (previous, Some(value)) = (before.get(key), after.get(key)) else {
            continue;
        };
        if previous == Some(value) {
            continue;
        }
        if let (Some(toml::Value::Table(previous)), toml::Value::Table(value), Some(written)) =
            (previous, value, written.as_table_like())
        {
            if !table.contains_key(key) {
                table.insert(key, empty(standard));
            }
            match table.get_mut(key) {
                Some(Item::Table(inner)) => {
                    let standard = !inner.is_dotted();
                    apply(inner, standard, previous, value, written);
                    continue;
                }
                Some(Item::Value(Value::InlineTable(inner))) => {
                    let closing = last_suffix(inner);
                    apply(inner, false, previous, value, written);
                    move_suffix(inner, closing);
                    continue;
                }
                _ => {}
            }
        }
        match table.get_mut(key) {
            Some(Item::Value(old)) => {
                let decor = old.decor().clone();
                *old = inline(written);
                *old.decor_mut() = decor;
            }
            _ => {
                table.insert(key, fresh(written, standard));
            }
        }
    }
}

/// The last key of an inline table and the space after its value, before the closing brace.
fn last_suffix(table: &InlineTable) -> Option<(String, RawString)> {
    let (key, value) = table.iter().last()?;
    Some((key.to_owned(), value.decor().suffix()?.clone()))
}

/// Keep the space before an inline table's closing brace after a key is added or removed at
/// its end, so `{ a = 1 }` gains `b` as `{ a = 1, b = 2 }`.
fn move_suffix(table: &mut InlineTable, closing: Option<(String, RawString)>) {
    let Some((last, suffix)) = closing else {
        return;
    };
    if table.iter().last().is_none_or(|(key, _)| key == last) {
        return;
    }
    if let Some(value) = table.get_mut(&last) {
        value.decor_mut().set_suffix("");
    }
    if let Some((_, value)) = table.iter_mut().last() {
        value.decor_mut().set_suffix(suffix);
    }
}

fn empty(standard: bool) -> Item {
    if standard {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    } else {
        Item::Value(Value::InlineTable(InlineTable::new()))
    }
}

/// `written` as a new item of a table that has a header when `standard` is set, free of
/// the layout of the document it was written in.
fn fresh(written: &Item, standard: bool) -> Item {
    if !standard {
        return Item::Value(inline(written));
    }
    match written {
        Item::Table(entries) => {
            let mut table = Table::new();
            table.set_implicit(true);
            for (key, item) in entries.iter() {
                table.insert(key, fresh(item, true));
            }
            Item::Table(table)
        }
        Item::ArrayOfTables(entries) => {
            let mut tables = ArrayOfTables::new();
            for entry in entries.iter() {
                if let Item::Table(table) = fresh(&Item::Table(entry.clone()), true) {
                    tables.push(table);
                }
            }
            Item::ArrayOfTables(tables)
        }
        item => Item::Value(inline(item)),
    }
}

fn inline(written: &Item) -> Value {
    let mut value = match written {
        Item::Value(value) => value.clone(),
        Item::Table(entries) => entries
            .iter()
            .map(|(key, item)| (key, inline(item)))
            .collect::<InlineTable>()
            .into(),
        Item::ArrayOfTables(entries) => entries
            .iter()
            .map(|entry| inline(&Item::Table(entry.clone())))
            .collect::<Array>()
            .into(),
        Item::None => Value::from(""),
    };
    value.decor_mut().clear();
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"# Dupin, as I like it.

hotkey   =   "alt+space"   # muscle memory
show_recent = true
dictionary_keywords = [
    "define",  # the one I use
    "word",
]
window_management = true

# Shorter results.
file_limit = 37

theme = { corner_radius = 20, panel_width = 700 }

[bindings."cmd+shift+v"]   # clipboard
target = "command"
command = "clipboard"

# Keep for later.
[bindings."alt+cmd+t"]
target = "script"
script = "todo.sh"

[aliases]
g = "web:g"   # search
cal = "calendar"

[clipboard]
# A long memory.
max_entries = 900
"#;

    const TODO: &str =
        "# Keep for later.\n[bindings.\"alt+cmd+t\"]\ntarget = \"script\"\nscript = \"todo.sh\"\n\n";

    fn parse(source: &str) -> Config {
        Config::parse(source).unwrap()
    }

    fn save(source: &str, change: impl FnOnce(&mut Config)) -> String {
        let before = parse(source);
        let mut after = before.clone();
        change(&mut after);
        let edited = edit(source, &before, &after).unwrap();
        assert_eq!(parse(&edited), after);
        edited
    }

    #[test]
    fn saves_change_only_the_values_they_touch() {
        let mut expected = SOURCE.to_owned();
        let edited = save(SOURCE, |config| config.file_limit = 50);
        expected = expected.replace("file_limit = 37", "file_limit = 50");
        assert_eq!(edited, expected);

        let edited = save(&edited, |config| config.hotkey = "ctrl+space".into());
        expected = expected.replace(r#""alt+space""#, r#""ctrl+space""#);
        assert_eq!(edited, expected);

        let edited = save(&edited, |config| config.theme.panel_width = 800);
        expected = expected.replace("panel_width = 700", "panel_width = 800");
        assert_eq!(edited, expected);

        let edited = save(&edited, |config| config.clipboard.max_entries = 1000);
        expected = expected.replace("max_entries = 900", "max_entries = 1000");
        assert_eq!(edited, expected);

        let edited = save(&edited, |config| {
            config.bindings.get_mut("cmd+shift+v").unwrap().clone_from(
                &parse("[bindings.x]\ntarget = \"command\"\ncommand = \"snippets\"\n").bindings["x"],
            );
        });
        expected = expected.replace(r#"command = "clipboard""#, r#"command = "snippets""#);
        assert_eq!(edited, expected);

        assert_eq!(save(&edited, |_| {}), expected);
    }

    #[test]
    fn removing_a_binding_or_an_alias_removes_only_its_own_lines() {
        let edited = save(SOURCE, |config| {
            config.bindings.remove("alt+cmd+t");
        });
        assert_eq!(edited, SOURCE.replace(TODO, ""));

        let edited = save(SOURCE, |config| {
            config.aliases.0.remove("cal");
        });
        assert_eq!(edited, SOURCE.replace("cal = \"calendar\"\n", ""));

        let edited = save(SOURCE, |config| config.aliases = Default::default());
        assert_eq!(
            edited,
            SOURCE.replace("[aliases]\ng = \"web:g\"   # search\ncal = \"calendar\"\n\n", "")
        );
    }

    #[test]
    fn a_key_joins_the_table_that_holds_it_or_a_new_one() {
        let edited = save(SOURCE, |config| config.clipboard.include_images = false);
        assert_eq!(edited, format!("{SOURCE}include_images = false\n"));

        let edited = save(SOURCE, |config| {
            config.calendar.enabled = !config.calendar.enabled
        });
        assert_eq!(
            edited,
            format!(
                "{SOURCE}\n[calendar]\nenabled = {}\n",
                parse(&edited).calendar.enabled
            )
        );

        let edited = save(SOURCE, |config| config.theme.font_size = 14);
        assert_eq!(
            edited,
            SOURCE.replace("panel_width = 700 }", "panel_width = 700, font_size = 14 }")
        );

        let added = "[bindings.\"ctrl+alt+n\"]\ntarget = \"script\"\nscript = \"notes.sh\"\n";
        let edited = save(SOURCE, |config| {
            config.bindings.extend(parse(added).bindings);
        });
        assert_eq!(edited, SOURCE.replace(TODO, &format!("{TODO}{added}\n")));
    }

    #[test]
    fn a_new_file_holds_only_what_changed() {
        let edited = save("", |config| config.file_limit = 50);
        assert_eq!(edited, "file_limit = 50\n");
        let edited = save(&edited, |config| {
            config.bindings = parse(TODO).bindings;
        });
        assert_eq!(
            edited,
            "file_limit = 50\n\n[bindings.\"alt+cmd+t\"]\ntarget = \"script\"\nscript = \"todo.sh\"\n"
        );
    }
}
