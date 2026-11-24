//! Desktop entries (`.desktop` files) as the freedesktop.org Desktop Entry Specification
//! defines them: the `[Desktop Entry]` group's keys, localized by the user's locale, and the
//! `Exec` command line with its field codes.
//! <https://specifications.freedesktop.org/desktop-entry-spec/latest/>

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The keys of an entry's `[Desktop Entry]` group that Dupin reads, localized.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// The untranslated `Name`, when the localized one differs.
    pub default_name: Option<String>,
    pub generic_name: Option<String>,
    pub keywords: Vec<String>,
    pub exec: Option<String>,
    pub icon: Option<String>,
    pub working_dir: Option<PathBuf>,
    pub terminal: bool,
}

/// The user's locale as the specification matches it: `lang_COUNTRY.ENCODING@MODIFIER`, from
/// `LC_ALL`, `LC_MESSAGES` or `LANG`, the first one set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Locale {
    lang: Option<String>,
    country: Option<String>,
    modifier: Option<String>,
}

impl Locale {
    pub fn from_env() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
            .map(|value| Self::parse(&value))
            .unwrap_or_default()
    }

    fn parse(value: &str) -> Self {
        if value == "C" || value == "POSIX" || value.starts_with("C.") {
            return Self::default();
        }
        let (rest, modifier) = match value.split_once('@') {
            Some((rest, modifier)) => (rest, Some(modifier.to_owned())),
            None => (value, None),
        };
        let rest = rest.split('.').next().unwrap_or_default();
        let (lang, country) = match rest.split_once('_') {
            Some((lang, country)) => (lang, Some(country.to_owned())),
            None => (rest, None),
        };
        Self {
            lang: (!lang.is_empty()).then(|| lang.to_owned()),
            country,
            modifier,
        }
    }

    /// The locale suffixes to try, most specific first: `lang_COUNTRY@MODIFIER`,
    /// `lang_COUNTRY`, `lang@MODIFIER`, `lang`.
    fn candidates(&self) -> Vec<String> {
        let Some(lang) = &self.lang else { return Vec::new() };
        let mut out = Vec::new();
        if let (Some(country), Some(modifier)) = (&self.country, &self.modifier) {
            out.push(format!("{lang}_{country}@{modifier}"));
        }
        if let Some(country) = &self.country {
            out.push(format!("{lang}_{country}"));
        }
        if let Some(modifier) = &self.modifier {
            out.push(format!("{lang}@{modifier}"));
        }
        out.push(lang.clone());
        out
    }
}

/// The desktops `OnlyShowIn` and `NotShowIn` name, from `XDG_CURRENT_DESKTOP`.
pub fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| {
            v.split(':')
                .filter(|d| !d.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// An application entry the menu shows, or `None` for a link, a directory, a hidden entry or
/// one for another desktop.
pub fn parse(text: &str, locale: &Locale, desktops: &[String]) -> Option<Entry> {
    let keys = group(text);
    let get = |key: &str| keys.get(key).map(String::as_str);
    let localized = |key: &str| {
        locale
            .candidates()
            .iter()
            .find_map(|suffix| keys.get(&format!("{key}[{suffix}]")))
            .or_else(|| keys.get(key))
            .map(|value| unescape(value))
    };
    let flag = |key: &str| get(key).is_some_and(|v| v.trim() == "true");
    if get("Type").map(str::trim) != Some("Application") || flag("Hidden") || flag("NoDisplay") {
        return None;
    }
    let listed = |key: &str| get(key).map(|v| split_list(&unescape(v)));
    let only = listed("OnlyShowIn");
    let not = listed("NotShowIn").unwrap_or_default();
    let shown = desktops
        .iter()
        .find_map(|d| {
            if only.as_ref().is_some_and(|only| only.contains(d)) {
                Some(true)
            } else if not.contains(d) {
                Some(false)
            } else {
                None
            }
        })
        .unwrap_or(only.is_none());
    if !shown {
        return None;
    }
    if let Some(try_exec) = get("TryExec")
        && find_executable(&unescape(try_exec)).is_none()
    {
        return None;
    }
    let name = localized("Name").filter(|n| !n.trim().is_empty())?;
    let default_name = get("Name").map(unescape).filter(|n| *n != name);
    Some(Entry {
        name,
        default_name,
        generic_name: localized("GenericName").filter(|n| !n.trim().is_empty()),
        keywords: localized("Keywords").map(|k| split_list(&k)).unwrap_or_default(),
        exec: get("Exec").map(unescape),
        icon: get("Icon").map(unescape).filter(|i| !i.is_empty()),
        working_dir: get("Path")
            .map(unescape)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from),
        terminal: flag("Terminal"),
    })
}

/// The `[Desktop Entry]` group's keys: the first value of each, with surrounding whitespace off.
fn group(text: &str) -> HashMap<String, String> {
    let mut keys = HashMap::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if inside {
                break;
            }
            inside = line == "[Desktop Entry]";
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            keys.entry(key.trim().to_owned())
                .or_insert_with(|| value.trim().to_owned());
        }
    }
    keys
}

/// A string value with its escapes (`\s`, `\n`, `\t`, `\r`, `\\`) resolved. `\;` stays, for the
/// list splitter to read.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// A `;`-separated list, with `\;` as a literal semicolon and empty items dropped.
fn split_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&';') => {
                current.push(';');
                chars.next();
            }
            ';' => items.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    items.push(current);
    items
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

/// `Exec` split into arguments with its field codes expanded for launching `entry` (at
/// `path`) without files: `%f`, `%F`, `%u`, `%U` and the deprecated codes go, `%i` becomes
/// `--icon <Icon>`, `%c` the name, `%k` the entry's path and `%%` a percent sign. `None` when
/// the line doesn't parse: an unclosed quote, or a reserved character outside quotes.
pub fn command(exec: &str, entry: &Entry, path: &Path) -> Option<Vec<OsString>> {
    let mut args = Vec::new();
    for (word, quoted) in words(exec)? {
        if quoted {
            args.push(OsString::from(word.replace("%%", "%")));
            continue;
        }
        match word.as_str() {
            "%f" | "%F" | "%u" | "%U" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
            "%i" => {
                if let Some(icon) = &entry.icon {
                    args.push("--icon".into());
                    args.push(icon.into());
                }
            }
            _ => {
                let mut expanded = String::new();
                let mut chars = word.chars();
                while let Some(c) = chars.next() {
                    if c != '%' {
                        expanded.push(c);
                        continue;
                    }
                    match chars.next() {
                        Some('%') => expanded.push('%'),
                        Some('c') => expanded.push_str(&entry.name),
                        Some('k') => expanded.push_str(&path.to_string_lossy()),
                        // Codes that expand to files, or to nothing, inside a word.
                        Some(_) | None => {}
                    }
                }
                args.push(expanded.into());
            }
        }
    }
    (!args.is_empty()).then_some(args)
}

/// The words of an `Exec` line and whether each was quoted. Inside double quotes, `\"`, `` \` ``,
/// `\$` and `\\` are escapes.
fn words(exec: &str) -> Option<Vec<(String, bool)>> {
    let mut words = Vec::new();
    let mut chars = exec.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
            chars.next();
        }
        let Some(&first) = chars.peek() else { break };
        if first == '"' {
            chars.next();
            let mut word = String::new();
            loop {
                match chars.next()? {
                    '"' => break,
                    '\\' => match chars.next()? {
                        c @ ('"' | '`' | '$' | '\\') => word.push(c),
                        c => {
                            word.push('\\');
                            word.push(c);
                        }
                    },
                    c => word.push(c),
                }
            }
            words.push((word, true));
        } else {
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if c == ' ' || c == '\t' {
                    break;
                }
                if matches!(
                    c,
                    '"' | '\''
                        | '`'
                        | '$'
                        | '\\'
                        | '>'
                        | '<'
                        | '~'
                        | '|'
                        | '&'
                        | ';'
                        | '*'
                        | '?'
                        | '#'
                        | '('
                        | ')'
                ) {
                    return None;
                }
                word.push(c);
                chars.next();
            }
            words.push((word, false));
        }
    }
    Some(words)
}

/// `program` as an absolute path, or found on `PATH`, when it is an executable file.
pub fn find_executable(program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |path: &Path| {
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        let path = PathBuf::from(program);
        return executable(&path).then_some(path);
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|path| executable(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ja() -> Locale {
        Locale::parse("ja_JP.UTF-8")
    }

    #[test]
    fn reads_the_localized_name_and_keeps_the_default_as_an_alias() {
        let entry = parse(
            "[Desktop Entry]\nType=Application\nName=Files\nName[ja]=ファイル\nKeywords=folder;manager;\nKeywords[ja]=フォルダ;\nExec=nautilus --new-window %U\n",
            &ja(),
            &[],
        )
        .unwrap();
        assert_eq!(entry.name, "ファイル");
        assert_eq!(entry.default_name.as_deref(), Some("Files"));
        assert_eq!(entry.keywords, ["フォルダ"]);
    }

    #[test]
    fn the_most_specific_locale_wins() {
        let text =
            "[Desktop Entry]\nType=Application\nName=Color\nName[en]=Color\nName[en_GB]=Color (GB)\nExec=x\n";
        let entry = parse(text, &Locale::parse("en_GB.UTF-8"), &[]).unwrap();
        assert_eq!(entry.name, "Color (GB)");
        let entry = parse(text, &Locale::parse("en_US.UTF-8"), &[]).unwrap();
        assert_eq!(entry.name, "Color");
        let entry = parse(text, &Locale::parse("C.UTF-8"), &[]).unwrap();
        assert_eq!(entry.name, "Color");
    }

    #[test]
    fn hidden_links_and_other_desktops_entries_are_skipped() {
        let base = "[Desktop Entry]\nName=X\nExec=x\n";
        assert!(parse(&format!("{base}Type=Link\n"), &ja(), &[]).is_none());
        assert!(parse(&format!("{base}Type=Application\nNoDisplay=true\n"), &ja(), &[]).is_none());
        assert!(parse(&format!("{base}Type=Application\nHidden=true\n"), &ja(), &[]).is_none());
        let xfce = ["XFCE".to_owned()];
        assert!(
            parse(
                &format!("{base}Type=Application\nOnlyShowIn=GNOME;\n"),
                &ja(),
                &xfce
            )
            .is_none()
        );
        assert!(
            parse(
                &format!("{base}Type=Application\nOnlyShowIn=XFCE;\n"),
                &ja(),
                &xfce
            )
            .is_some()
        );
        assert!(
            parse(
                &format!("{base}Type=Application\nNotShowIn=XFCE;\n"),
                &ja(),
                &xfce
            )
            .is_none()
        );
        let gnome_unity = ["GNOME".to_owned(), "Unity".to_owned()];
        assert!(
            parse(
                &format!("{base}Type=Application\nOnlyShowIn=GNOME;\nNotShowIn=Unity;\n"),
                &ja(),
                &gnome_unity
            )
            .is_some()
        );
        assert!(
            parse(
                &format!("{base}Type=Application\nOnlyShowIn=Unity;\nNotShowIn=GNOME;\n"),
                &ja(),
                &gnome_unity
            )
            .is_none()
        );
        assert!(
            parse(
                &format!("{base}Type=Application\nTryExec=/nonexistent/dupin-test\n"),
                &ja(),
                &[]
            )
            .is_none()
        );
    }

    #[test]
    fn only_the_desktop_entry_group_counts() {
        let entry = parse(
            "[Desktop Entry]\nType=Application\nName=Editor\nExec=editor\n\n[Desktop Action new]\nName=New Window\nExec=editor --new\n",
            &ja(),
            &[],
        )
        .unwrap();
        assert_eq!(entry.name, "Editor");
        assert_eq!(entry.exec.as_deref(), Some("editor"));
    }

    #[test]
    fn expands_field_codes() {
        let entry = Entry {
            name: "Editor".into(),
            icon: Some("editor-icon".into()),
            ..Entry::default()
        };
        let path = Path::new("/usr/share/applications/editor.desktop");
        let args = command(r#"editor %U --name=%c %i "100%%" %k"#, &entry, path).unwrap();
        assert_eq!(
            args,
            [
                "editor",
                "--name=Editor",
                "--icon",
                "editor-icon",
                "100%",
                "/usr/share/applications/editor.desktop"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn quoted_arguments_keep_spaces_and_escapes() {
        let entry = Entry::default();
        let path = Path::new("/x.desktop");
        let args = command(r#""/opt/My App/run" "say \"hi\"" plain"#, &entry, path).unwrap();
        assert_eq!(
            args,
            ["/opt/My App/run", r#"say "hi""#, "plain"].map(OsString::from)
        );
        assert!(command(r#""unclosed"#, &entry, path).is_none());
        assert!(command("sh -c a|b", &entry, path).is_none());
    }

    #[test]
    fn string_escapes_and_lists() {
        assert_eq!(unescape(r"a\sb\\c"), r"a b\c");
        assert_eq!(split_list(r"one;two\;three;;"), ["one", "two;three"]);
    }
}
