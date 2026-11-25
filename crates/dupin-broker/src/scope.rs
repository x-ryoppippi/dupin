use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

macro_rules! string_type {
    ($name:ident, $check:expr) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                let check: fn(&str) -> Result<(), String> = $check;
                check(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;
            fn try_from(value: String) -> Result<Self, String> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_type!(PackageId, |value| {
    if value.is_empty() || value.len() > 512 {
        return Err("a package ID is 1–512 bytes".into());
    }
    if value.chars().any(char::is_control) {
        return Err("a package ID has no control characters".into());
    }
    Ok(())
});

string_type!(Revision, |value| {
    let valid = (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":_+/=-".contains(&b));
    if valid {
        Ok(())
    } else {
        Err("a revision is 1–128 letters, digits or `:_+/=-`".into())
    }
});

string_type!(ScopeName, |value| {
    let valid = (1..=64).contains(&value.len())
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    if valid {
        Ok(())
    } else {
        Err("a name is 1–64 letters, digits, `.`, `_` or `-`, starting with a letter or digit".into())
    }
});

/// An HTTPS origin: a lowercase DNS host and an optional port other than 443.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Origin {
    host: String,
    port: Option<u16>,
}

impl Origin {
    /// Parses a manifest origin, which must already be in canonical form.
    pub fn parse(value: &str) -> Result<Self, String> {
        let origin = Self::of_url(value)?;
        if origin.to_string() != value {
            return Err(format!("write the origin as `{origin}`"));
        }
        Ok(origin)
    }

    /// The canonical origin of a request URL: the scheme and host are case-insensitive and
    /// port 443 is implied.
    pub fn of_url(url: &str) -> Result<Self, String> {
        let Some(rest) = url
            .get(..8)
            .filter(|scheme| scheme.eq_ignore_ascii_case("https://"))
            .map(|_| &url[8..])
        else {
            return Err("only `https://` URLs are allowed".into());
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        if authority.contains('@') {
            return Err("URLs can't carry a user name or password".into());
        }
        if authority.starts_with('[') {
            return Err("IPv6 literal hosts aren't supported".into());
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                let port: u16 = port
                    .parse()
                    .ok()
                    .filter(|&port| port != 0)
                    .ok_or("the port must be 1–65535")?;
                (host, (port != 443).then_some(port))
            }
            None => (authority, None),
        };
        let host = host.to_ascii_lowercase();
        let valid_label = |label: &str| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        };
        if host.is_empty() || host.len() > 253 || !host.split('.').all(valid_label) {
            return Err("the host must be a DNS name of letters, digits, `-` and `.`".into());
        }
        Ok(Self { host, port })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port.unwrap_or(443)
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "https://{}", self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

impl TryFrom<String> for Origin {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Self::parse(&value)
    }
}

impl From<Origin> for String {
    fn from(value: Origin) -> String {
        value.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    pub const ALL: [Self; 6] = [
        Self::Get,
        Self::Head,
        Self::Post,
        Self::Put,
        Self::Patch,
        Self::Delete,
    ];

    /// Methods that change something on the server, so they need a user activation.
    pub fn mutates(self) -> bool {
        !matches!(self, Self::Get | Self::Head)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilesKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilesAccess {
    Read,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardAccess {
    Read,
    Write,
    Watch,
    /// Pasting a row's text into the app in front of Dupin, which Dupin does by posting ⌘V.
    Paste,
}

impl ClipboardAccess {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Watch => "watch",
            Self::Paste => "paste",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowsAccess {
    /// Listing displays and reading the focused window's frame and app.
    Read,
    /// Moving and resizing the focused window.
    Arrange,
}

impl WindowsAccess {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Arrange => "arrange",
        }
    }
}

/// A kind of row Dupin's own sources show, whose fields a package may be let see.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowKind {
    Text,
    Url,
    File,
    App,
}

impl RowKind {
    pub const ALL: [Self; 4] = [Self::Text, Self::Url, Self::File, Self::App];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Url => "url",
            Self::File => "file",
            Self::App => "app",
        }
    }

    /// The fields a row of this kind has: its title, and the value it stands for.
    pub fn fields(self) -> &'static [RowField] {
        match self {
            Self::Text => &[RowField::Title, RowField::Text],
            Self::Url => &[RowField::Title, RowField::Url],
            Self::File | Self::App => &[RowField::Title, RowField::Name, RowField::Path],
        }
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Text => "text results",
            Self::Url => "links",
            Self::File => "files and folders",
            Self::App => "apps",
        }
    }
}

/// One field of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowField {
    Title,
    Text,
    Url,
    /// A file's or app's name in its folder.
    Name,
    /// A file's or app's full path.
    Path,
}

impl RowField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Text => "text",
            Self::Url => "url",
            Self::Name => "name",
            Self::Path => "path",
        }
    }

    fn described(self) -> &'static str {
        match self {
            Self::Title => "titles",
            Self::Text => "text",
            Self::Url => "addresses",
            Self::Name => "names",
            Self::Path => "full paths",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageOp {
    Read,
    Write,
    Delete,
}

/// An OS-level grant a scope also needs, which macOS (or another platform) decides for the
/// whole app. Settings → Permissions lists the packages that declare it beside the built-in
/// features that use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OsPermission {
    /// Full access to calendar events.
    Calendars,
    /// Reading the general pasteboard without the system's paste prompt.
    Pasteboard,
    /// Posting key events, such as ⌘V into the front app, and reading and moving other apps'
    /// windows.
    Accessibility,
}

impl OsPermission {
    pub const ALL: [Self; 3] = [Self::Calendars, Self::Pasteboard, Self::Accessibility];

    /// The name System Settings lists it under.
    pub fn name(self) -> &'static str {
        match self {
            Self::Calendars => "Calendars",
            Self::Pasteboard => "Paste from Other Apps",
            Self::Accessibility => "Accessibility",
        }
    }
}

/// One grantable unit of a declaration. A [`ScopeKey`] names it; the rest bounds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Scope {
    Http {
        origin: Origin,
        methods: BTreeSet<Method>,
    },
    Files {
        name: ScopeName,
        kind: FilesKind,
        access: BTreeSet<FilesAccess>,
    },
    Exec {
        name: ScopeName,
        args: Vec<String>,
        output: bool,
    },
    Script {
        name: ScopeName,
        output: bool,
    },
    Storage {
        quota_bytes: u64,
        retention_days: Option<u32>,
    },
    Clipboard {
        access: ClipboardAccess,
    },
    Dictionary,
    Calendar {
        past_days: u32,
        future_days: u32,
    },
    Windows {
        access: WindowsAccess,
    },
    /// Fields of the rows of one kind that Dupin's own sources show, which the package's
    /// filters and kind actions receive. Another package's rows never reach it.
    Rows {
        kind: RowKind,
        fields: BTreeSet<RowField>,
    },
    /// What the user types in the searches the package's matchers are composed into.
    Query,
}

impl Scope {
    pub fn key(&self) -> ScopeKey {
        match self {
            Self::Http { origin, .. } => ScopeKey::Http(origin.clone()),
            Self::Files { name, .. } => ScopeKey::Files(name.clone()),
            Self::Exec { name, .. } => ScopeKey::Exec(name.clone()),
            Self::Script { name, .. } => ScopeKey::Script(name.clone()),
            Self::Storage { .. } => ScopeKey::Storage,
            Self::Clipboard { access } => ScopeKey::Clipboard(*access),
            Self::Dictionary => ScopeKey::Dictionary,
            Self::Calendar { .. } => ScopeKey::Calendar,
            Self::Windows { access } => ScopeKey::Windows(*access),
            Self::Rows { kind, .. } => ScopeKey::Rows(*kind),
            Self::Query => ScopeKey::Query,
        }
    }

    /// Whether an approval of `self` also approves `other`, so a revision that declares
    /// `other` keeps the grant without asking again.
    pub fn covers(&self, other: &Scope) -> bool {
        match (self, other) {
            (
                Self::Http { origin, methods },
                Self::Http {
                    origin: o,
                    methods: m,
                },
            ) => origin == o && methods.is_superset(m),
            (
                Self::Files { name, kind, access },
                Self::Files {
                    name: n,
                    kind: k,
                    access: a,
                },
            ) => name == n && kind == k && access.is_superset(a),
            (
                Self::Exec { name, args, output },
                Self::Exec {
                    name: n,
                    args: a,
                    output: o,
                },
            ) => name == n && args == a && (*output || !o),
            (Self::Script { name, output }, Self::Script { name: n, output: o }) => {
                name == n && (*output || !o)
            }
            (
                Self::Storage {
                    quota_bytes,
                    retention_days,
                },
                Self::Storage {
                    quota_bytes: q,
                    retention_days: r,
                },
            ) => {
                quota_bytes >= q
                    && match (retention_days, r) {
                        (None, _) => true,
                        (Some(_), None) => false,
                        (Some(approved), Some(declared)) => approved >= declared,
                    }
            }
            (Self::Clipboard { access }, Self::Clipboard { access: a }) => access == a,
            (Self::Dictionary, Self::Dictionary) => true,
            (
                Self::Calendar {
                    past_days,
                    future_days,
                },
                Self::Calendar {
                    past_days: p,
                    future_days: f,
                },
            ) => past_days >= p && future_days >= f,
            (Self::Windows { access }, Self::Windows { access: a }) => access == a,
            (Self::Rows { kind, fields }, Self::Rows { kind: k, fields: f }) => {
                kind == k && fields.is_superset(f)
            }
            (Self::Query, Self::Query) => true,
            _ => false,
        }
    }

    pub fn os_permission(&self) -> Option<OsPermission> {
        match self {
            Self::Calendar { .. } => Some(OsPermission::Calendars),
            Self::Clipboard {
                access: ClipboardAccess::Read | ClipboardAccess::Watch,
            } => Some(OsPermission::Pasteboard),
            Self::Clipboard {
                access: ClipboardAccess::Paste,
            }
            | Self::Windows { .. } => Some(OsPermission::Accessibility),
            _ => None,
        }
    }

    /// Files and executable profiles are bound, at consent, to a path the user picks.
    pub fn needs_binding(&self) -> bool {
        matches!(self, Self::Files { .. } | Self::Exec { .. })
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn list<T>(items: impl IntoIterator<Item = T>, name: impl Fn(T) -> &'static str) -> String {
            let names: Vec<_> = items.into_iter().map(name).collect();
            match names.as_slice() {
                [rest @ .., last] if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
                _ => names.concat(),
            }
        }
        fn days(count: u32) -> String {
            if count == 1 {
                "1 day".into()
            } else {
                format!("{count} days")
            }
        }
        match self {
            Self::Http { origin, methods } => {
                write!(f, "{} requests to {origin}", list(methods, |m| m.as_str()))
            }
            Self::Files { name, kind, access } => {
                let kind = match kind {
                    FilesKind::File => "file",
                    FilesKind::Directory => "folder",
                };
                let access = list(access, |a| match a {
                    FilesAccess::Read => "read",
                    FilesAccess::List => "list",
                });
                write!(f, "{access} the {kind} you pick for “{name}”")
            }
            Self::Exec { name, args, output } => {
                write!(f, "run the program you pick for “{name}”")?;
                for (index, arg) in args.iter().enumerate() {
                    f.write_str(if index == 0 { " with " } else { " " })?;
                    write!(f, "{arg:?}")?;
                }
                if *output {
                    f.write_str(" and read its output")?;
                }
                Ok(())
            }
            Self::Script { name, output } => {
                write!(f, "run your script command “{name}”")?;
                if *output {
                    f.write_str(" and read its output")?;
                }
                Ok(())
            }
            Self::Storage {
                quota_bytes,
                retention_days,
            } => {
                write!(f, "keep up to {quota_bytes} bytes of encrypted data")?;
                if let Some(count) = retention_days {
                    write!(f, " for {}", days(*count))?;
                }
                Ok(())
            }
            Self::Clipboard { access } => match access {
                ClipboardAccess::Read => f.write_str("read the clipboard"),
                ClipboardAccess::Write => f.write_str("write to the clipboard"),
                ClipboardAccess::Watch => f.write_str("see each new clipboard entry"),
                ClipboardAccess::Paste => f.write_str("paste into the app you were using"),
            },
            Self::Dictionary => f.write_str("look words up in the system dictionaries"),
            Self::Calendar {
                past_days,
                future_days,
            } => write!(
                f,
                "read calendar events from {} back to {} ahead",
                days(*past_days),
                days(*future_days)
            ),
            Self::Windows { access } => match access {
                WindowsAccess::Read => f.write_str("see your displays and the focused window"),
                WindowsAccess::Arrange => f.write_str("move and resize the focused window"),
            },
            Self::Rows { kind, fields } => write!(
                f,
                "see the {} of {} Dupin finds, where you compose its filters or actions",
                list(fields, |field| field.described()),
                kind.plural()
            ),
            Self::Query => f.write_str("see what you type in the searches you compose its matchers into"),
        }
    }
}

/// The name of one grant: an HTTP origin, a named files or exec scope, a script target, or a
/// single-valued capability. Written as `http:https://example.com`, `files:notes`,
/// `exec:git-status`, `script:open-in-zed`, `storage`, `clipboard:read`, `dictionary`,
/// `calendar` or `windows:arrange` in grant files.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ScopeKey {
    Http(Origin),
    Files(ScopeName),
    Exec(ScopeName),
    Script(ScopeName),
    Storage,
    Clipboard(ClipboardAccess),
    Dictionary,
    Calendar,
    Windows(WindowsAccess),
    Rows(RowKind),
    Query,
}

impl fmt::Display for ScopeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(origin) => write!(f, "http:{origin}"),
            Self::Files(name) => write!(f, "files:{name}"),
            Self::Exec(name) => write!(f, "exec:{name}"),
            Self::Script(name) => write!(f, "script:{name}"),
            Self::Storage => f.write_str("storage"),
            Self::Clipboard(access) => write!(f, "clipboard:{}", access.as_str()),
            Self::Dictionary => f.write_str("dictionary"),
            Self::Calendar => f.write_str("calendar"),
            Self::Windows(access) => write!(f, "windows:{}", access.as_str()),
            Self::Rows(kind) => write!(f, "rows:{}", kind.as_str()),
            Self::Query => f.write_str("query"),
        }
    }
}

impl FromStr for ScopeKey {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, String> {
        let unknown = || format!("`{value}` isn't a scope");
        Ok(match value.split_once(':') {
            None => match value {
                "storage" => Self::Storage,
                "dictionary" => Self::Dictionary,
                "calendar" => Self::Calendar,
                "query" => Self::Query,
                _ => return Err(unknown()),
            },
            Some(("http", origin)) => Self::Http(Origin::parse(origin)?),
            Some(("files", name)) => Self::Files(ScopeName::new(name)?),
            Some(("exec", name)) => Self::Exec(ScopeName::new(name)?),
            Some(("script", name)) => Self::Script(ScopeName::new(name)?),
            Some(("clipboard", "read")) => Self::Clipboard(ClipboardAccess::Read),
            Some(("clipboard", "write")) => Self::Clipboard(ClipboardAccess::Write),
            Some(("clipboard", "watch")) => Self::Clipboard(ClipboardAccess::Watch),
            Some(("clipboard", "paste")) => Self::Clipboard(ClipboardAccess::Paste),
            Some(("windows", "read")) => Self::Windows(WindowsAccess::Read),
            Some(("windows", "arrange")) => Self::Windows(WindowsAccess::Arrange),
            Some(("rows", kind)) => Self::Rows(
                RowKind::ALL
                    .into_iter()
                    .find(|known| known.as_str() == kind)
                    .ok_or_else(unknown)?,
            ),
            Some(_) => return Err(unknown()),
        })
    }
}

impl TryFrom<String> for ScopeKey {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        value.parse()
    }
}

impl From<ScopeKey> for String {
    fn from(value: ScopeKey) -> String {
        value.to_string()
    }
}

/// Whether `text` shows as itself in the consent text: control, line-separator and invisible
/// formatting characters such as bidi overrides could forge or reorder the lines a user approves.
pub fn displayable(text: &str) -> bool {
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
