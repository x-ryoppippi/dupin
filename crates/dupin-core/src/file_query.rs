//! Pure parsing for the file-search query language.
//!
//! The original query remains the user-facing text. This module only separates ordinary search
//! terms from the supported semantic filters; platform-specific interpretation happens elsewhere.

use std::{error::Error, fmt, ops::Range, time::Duration};

/// A parsed file query. `terms` retain their spelling and order from the input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileQuery {
    pub terms: Vec<String>,
    /// Byte ranges of supported qualifier tokens in the original query, excluding whitespace.
    pub qualifier_ranges: Vec<Range<usize>>,
    pub kind: Option<FileKind>,
    pub extension: Option<String>,
    pub folder: Option<String>,
    pub modified_within: Option<Duration>,
}

impl FileQuery {
    /// Parse supported qualifiers while leaving ordinary text on the existing whitespace-based
    /// search path.
    pub fn parse(raw: &str) -> Result<Self, FileQueryError> {
        let mut query = Self::default();
        let mut offset = 0;

        while offset < raw.len() {
            let rest = &raw[offset..];
            let Some(first) = rest.chars().next() else {
                break;
            };
            if first.is_whitespace() {
                offset += first.len_utf8();
                continue;
            }

            if let Some((key, value_offset)) = qualifier_prefix(rest) {
                let (value, next_offset) = parse_value(raw, offset + value_offset)?;
                assign_qualifier(&mut query, key, &value)?;
                query.qualifier_ranges.push(offset..next_offset);
                offset = next_offset;
                continue;
            }

            let end = rest
                .char_indices()
                .find_map(|(index, character)| character.is_whitespace().then_some(index))
                .unwrap_or(rest.len());
            query.terms.push(rest[..end].to_owned());
            offset += end;
        }

        Ok(query)
    }
}

/// The file categories accepted by `kind:`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Pdf,
    Image,
    Text,
    Audio,
    Video,
    Folder,
    Archive,
    Application,
}

/// A syntax or validation error in a supported qualifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileQueryError {
    EmptyValue,
    UnterminatedQuote,
    TrailingQuotedValue,
    DuplicateQualifier,
    InvalidKind,
    InvalidExtension,
    InvalidFolder,
    InvalidModified,
}

impl FileQueryError {
    /// Return safe, static guidance for the invalid query.
    pub const fn message(&self) -> &'static str {
        match self {
            Self::EmptyValue => "Add a value after the qualifier, such as kind:pdf or ext:rs.",
            Self::UnterminatedQuote => "Close the quoted qualifier value with a double quote.",
            Self::TrailingQuotedValue => "Put whitespace after a closing quote in a qualifier value.",
            Self::DuplicateQualifier => "Use each file-search qualifier only once.",
            Self::InvalidKind => "Use kind:pdf, image, text, audio, video, folder, archive, or app.",
            Self::InvalidExtension => {
                "Use an extension made of letters, numbers, underscores, or hyphens, separated by dots."
            }
            Self::InvalidFolder => {
                "Use an absolute folder path, ~, or a path beginning with ~/; quote paths with spaces."
            }
            Self::InvalidModified => {
                "Use a positive duration such as modified:4h, modified:7d, or modified:2w."
            }
        }
    }
}

impl fmt::Display for FileQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl Error for FileQueryError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Qualifier {
    Kind,
    Extension,
    Folder,
    Modified,
}

fn qualifier_prefix(token: &str) -> Option<(Qualifier, usize)> {
    let (colon, _) = token
        .char_indices()
        .find(|(_, character)| *character == ':' || character.is_whitespace())?;
    if token.as_bytes()[colon] != b':' {
        return None;
    }

    let key = &token[..colon];
    let qualifier = if key.eq_ignore_ascii_case("kind") {
        Qualifier::Kind
    } else if key.eq_ignore_ascii_case("ext") {
        Qualifier::Extension
    } else if key.eq_ignore_ascii_case("in") {
        Qualifier::Folder
    } else if key.eq_ignore_ascii_case("modified") {
        Qualifier::Modified
    } else {
        return None;
    };

    Some((qualifier, colon + 1))
}

fn parse_value(raw: &str, value_offset: usize) -> Result<(String, usize), FileQueryError> {
    let rest = &raw[value_offset..];
    if rest.starts_with('"') {
        parse_quoted_value(raw, value_offset)
    } else {
        let mut value = String::new();
        let mut end = 0;
        let mut characters = rest.char_indices().peekable();
        while let Some((index, character)) = characters.next() {
            if character.is_whitespace() {
                break;
            }
            end = index + character.len_utf8();
            if character == '\\' {
                let Some((escaped_index, escaped)) = characters.next() else {
                    value.push('\\');
                    break;
                };
                end = escaped_index + escaped.len_utf8();
                if escaped == '"' || escaped == '\\' || escaped.is_whitespace() {
                    value.push(escaped);
                } else {
                    value.push('\\');
                    value.push(escaped);
                }
            } else {
                value.push(character);
            }
        }
        if value.is_empty() {
            return Err(FileQueryError::EmptyValue);
        }
        Ok((value, value_offset + end))
    }
}

fn parse_quoted_value(raw: &str, quote_offset: usize) -> Result<(String, usize), FileQueryError> {
    let mut value = String::new();
    let mut characters = raw[quote_offset + 1..].char_indices().peekable();

    while let Some((relative_offset, character)) = characters.next() {
        match character {
            '"' => {
                let end = quote_offset + 1 + relative_offset + character.len_utf8();
                if raw[end..]
                    .chars()
                    .next()
                    .is_some_and(|next| !next.is_whitespace())
                {
                    return Err(FileQueryError::TrailingQuotedValue);
                }
                if value.is_empty() {
                    return Err(FileQueryError::EmptyValue);
                }
                return Ok((value, end));
            }
            '\\' => {
                let Some((_, escaped)) = characters.next() else {
                    return Err(FileQueryError::UnterminatedQuote);
                };
                if escaped == '"' || escaped == '\\' || escaped.is_whitespace() {
                    value.push(escaped);
                } else {
                    value.push('\\');
                    value.push(escaped);
                }
            }
            other => value.push(other),
        }
    }

    Err(FileQueryError::UnterminatedQuote)
}

fn assign_qualifier(query: &mut FileQuery, qualifier: Qualifier, value: &str) -> Result<(), FileQueryError> {
    match qualifier {
        Qualifier::Kind => {
            if query.kind.is_some() {
                return Err(FileQueryError::DuplicateQualifier);
            }
            query.kind = Some(parse_kind(value)?);
        }
        Qualifier::Extension => {
            if query.extension.is_some() {
                return Err(FileQueryError::DuplicateQualifier);
            }
            query.extension = Some(parse_extension(value)?);
        }
        Qualifier::Folder => {
            if query.folder.is_some() {
                return Err(FileQueryError::DuplicateQualifier);
            }
            query.folder = Some(parse_folder(value)?);
        }
        Qualifier::Modified => {
            if query.modified_within.is_some() {
                return Err(FileQueryError::DuplicateQualifier);
            }
            query.modified_within = Some(parse_modified(value)?);
        }
    }
    Ok(())
}

fn parse_kind(value: &str) -> Result<FileKind, FileQueryError> {
    if value.chars().any(char::is_control) {
        return Err(FileQueryError::InvalidKind);
    }
    match value.to_ascii_lowercase().as_str() {
        "pdf" => Ok(FileKind::Pdf),
        "image" | "images" => Ok(FileKind::Image),
        "text" => Ok(FileKind::Text),
        "audio" => Ok(FileKind::Audio),
        "video" => Ok(FileKind::Video),
        "folder" | "directory" => Ok(FileKind::Folder),
        "archive" => Ok(FileKind::Archive),
        "app" | "application" => Ok(FileKind::Application),
        _ => Err(FileQueryError::InvalidKind),
    }
}

fn parse_extension(value: &str) -> Result<String, FileQueryError> {
    let extension = value.strip_prefix('.').unwrap_or(value);
    if extension.is_empty()
        || extension.split('.').any(|component| component.is_empty())
        || !extension
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err(FileQueryError::InvalidExtension);
    }
    Ok(extension.to_owned())
}

fn parse_folder(value: &str) -> Result<String, FileQueryError> {
    if value.chars().any(char::is_control)
        || !(value == "~" || value.starts_with("~/") || value.starts_with('/'))
    {
        return Err(FileQueryError::InvalidFolder);
    }
    Ok(value.to_owned())
}

fn parse_modified(value: &str) -> Result<Duration, FileQueryError> {
    let Some((unit, digits)) = value.chars().next_back().map(|unit| {
        let digits = &value[..value.len() - unit.len_utf8()];
        (unit, digits)
    }) else {
        return Err(FileQueryError::InvalidModified);
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(FileQueryError::InvalidModified);
    }
    let amount = digits
        .parse::<u64>()
        .map_err(|_| FileQueryError::InvalidModified)?;
    if amount == 0 {
        return Err(FileQueryError::InvalidModified);
    }
    let seconds_per_unit = match unit.to_ascii_lowercase() {
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        'w' => 7 * 24 * 60 * 60,
        _ => return Err(FileQueryError::InvalidModified),
    };
    let seconds = amount
        .checked_mul(seconds_per_unit)
        .ok_or(FileQueryError::InvalidModified)?;
    Ok(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_terms_and_all_qualifiers() {
        let raw = "draft KIND:PDF ext:.rs in:~/Work modified:7d report";
        let query = FileQuery::parse(raw).expect("valid file query");
        assert_eq!(query.terms, ["draft", "report"]);
        assert_eq!(
            query
                .qualifier_ranges
                .iter()
                .map(|range| &raw[range.clone()])
                .collect::<Vec<_>>(),
            ["KIND:PDF", "ext:.rs", "in:~/Work", "modified:7d"]
        );
        assert_eq!(query.kind, Some(FileKind::Pdf));
        assert_eq!(query.extension.as_deref(), Some("rs"));
        assert_eq!(query.folder.as_deref(), Some("~/Work"));
        assert_eq!(query.modified_within, Some(Duration::from_secs(7 * 24 * 60 * 60)));
    }

    #[test]
    fn quoted_unicode_folder_decodes_only_supported_escapes() {
        let raw = r#"in:"~/日本 語/猫\ folder/quote\"file\\name""#;
        let query = FileQuery::parse(raw).expect("valid quoted folder");
        assert_eq!(
            query.folder.as_deref(),
            Some("~/日本 語/猫 folder/quote\"file\\name")
        );
        let range = &query.qualifier_ranges[0];
        assert!(raw.is_char_boundary(range.start));
        assert!(raw.is_char_boundary(range.end));
        assert_eq!(&raw[range.clone()], raw);
    }

    #[test]
    fn qualifier_ranges_are_raw_utf8_byte_spans_after_multibyte_terms() {
        let raw = "猫 file in:\"~/日本 語/資料\" report";
        let query = FileQuery::parse(raw).expect("valid file query");
        let range = &query.qualifier_ranges[0];
        assert!(raw.is_char_boundary(range.start));
        assert!(raw.is_char_boundary(range.end));
        assert_eq!(&raw[range.clone()], "in:\"~/日本 語/資料\"");
        assert_eq!(query.terms, ["猫", "file", "report"]);
    }

    #[test]
    fn quoted_absolute_folder_can_contain_escaped_whitespace() {
        let query = FileQuery::parse(r#"in:"/Users/李 雷/Work\ Docs""#).expect("valid quoted folder");
        assert_eq!(query.folder.as_deref(), Some("/Users/李 雷/Work Docs"));

        let query = FileQuery::parse(r"in:~/Work\ Docs").expect("valid escaped folder");
        assert_eq!(query.folder.as_deref(), Some("~/Work Docs"));
    }

    #[test]
    fn unknown_colons_and_ordinary_quotes_apostrophes_and_backslashes_stay_literal() {
        let query = FileQuery::parse(r#"owner's name:"literal" C:\work\file foo:bar xkind:pdf"#)
            .expect("ordinary terms remain valid");
        assert_eq!(
            query.terms,
            [
                "owner's",
                "name:\"literal\"",
                r"C:\work\file",
                "foo:bar",
                "xkind:pdf"
            ]
        );
        assert_eq!(query.kind, None);
        assert_eq!(query.extension, None);
        assert_eq!(query.folder, None);
        assert_eq!(query.modified_within, None);
    }

    #[test]
    fn recognizes_kind_aliases_case_insensitively() {
        for (value, kind) in [
            ("pdf", FileKind::Pdf),
            ("Images", FileKind::Image),
            ("text", FileKind::Text),
            ("audio", FileKind::Audio),
            ("video", FileKind::Video),
            ("directory", FileKind::Folder),
            ("archive", FileKind::Archive),
            ("APPLICATION", FileKind::Application),
        ] {
            assert_eq!(
                FileQuery::parse(&format!("kind:{value}")).unwrap().kind,
                Some(kind)
            );
        }
    }

    #[test]
    fn validates_extensions_and_accepts_unicode_components() {
        let query = FileQuery::parse("ext:.résumé-2_part.β").expect("valid extension");
        assert_eq!(query.extension.as_deref(), Some("résumé-2_part.β"));
        for invalid in [".", "..rs", "rs.*", "rs/name", "rs\"name"] {
            assert_eq!(
                FileQuery::parse(&format!("ext:{invalid}")).unwrap_err(),
                FileQueryError::InvalidExtension,
                "{invalid:?}"
            );
        }
        assert_eq!(
            FileQuery::parse("ext:\"rs\nname\"").unwrap_err(),
            FileQueryError::InvalidExtension
        );
    }

    #[test]
    fn validates_folder_forms_and_controls() {
        for valid in ["~", "~/", "~/Work", "/", "/tmp/資料"] {
            assert_eq!(
                FileQuery::parse(&format!("in:{valid}"))
                    .unwrap()
                    .folder
                    .as_deref(),
                Some(valid)
            );
        }
        for invalid in ["relative", "~alice", "~\\Work"] {
            assert_eq!(
                FileQuery::parse(&format!("in:{invalid}")).unwrap_err(),
                FileQueryError::InvalidFolder,
                "{invalid:?}"
            );
        }
        assert_eq!(
            FileQuery::parse("in:\"/tmp\nname\"").unwrap_err(),
            FileQueryError::InvalidFolder
        );
    }

    #[test]
    fn parses_modified_duration_units_and_case() {
        for (value, seconds) in [
            ("1h", 60 * 60),
            ("2H", 2 * 60 * 60),
            ("3d", 3 * 24 * 60 * 60),
            ("4W", 4 * 7 * 24 * 60 * 60),
        ] {
            assert_eq!(
                FileQuery::parse(&format!("modified:{value}"))
                    .unwrap()
                    .modified_within,
                Some(Duration::from_secs(seconds))
            );
        }
    }

    #[test]
    fn rejects_invalid_modified_values_and_overflow() {
        for value in ["0d", "+1d", "-1d", "1.5d", "1m", "d", "18446744073709551615w"] {
            assert_eq!(
                FileQuery::parse(&format!("modified:{value}")).unwrap_err(),
                FileQueryError::InvalidModified,
                "{value:?}"
            );
        }
    }

    #[test]
    fn rejects_malformed_quoted_values_and_trailing_junk() {
        assert_eq!(
            FileQuery::parse("in:\"/Users/a path").unwrap_err(),
            FileQueryError::UnterminatedQuote
        );
        assert_eq!(
            FileQuery::parse("in:\"/Users/a\"tail").unwrap_err(),
            FileQueryError::TrailingQuotedValue
        );
        assert_eq!(
            FileQuery::parse("in:\"\"").unwrap_err(),
            FileQueryError::EmptyValue
        );
    }

    #[test]
    fn rejects_empty_invalid_and_duplicate_qualifiers() {
        for raw in ["kind:", "ext:", "in:", "modified:"] {
            assert_eq!(
                FileQuery::parse(raw).unwrap_err(),
                FileQueryError::EmptyValue,
                "{raw}"
            );
        }
        for raw in [
            "kind:document",
            "kind:pdf kind:image",
            "ext:rs EXT:md",
            "in:~/a in:~/b",
            "modified:1d modified:2d",
        ] {
            let error = FileQuery::parse(raw).unwrap_err();
            assert!(
                matches!(
                    error,
                    FileQueryError::InvalidKind | FileQueryError::DuplicateQualifier
                ),
                "{raw}: {error}"
            );
        }
        assert_eq!(
            FileQuery::parse("ext:rs ext:md").unwrap_err(),
            FileQueryError::DuplicateQualifier
        );
        assert_eq!(
            FileQuery::parse("kind:pdf kind:image").unwrap_err(),
            FileQueryError::DuplicateQualifier
        );
        assert_eq!(
            FileQuery::parse("in:~/a in:~/b").unwrap_err(),
            FileQueryError::DuplicateQualifier
        );
        assert_eq!(
            FileQuery::parse("modified:1d modified:2d").unwrap_err(),
            FileQueryError::DuplicateQualifier
        );
    }

    #[test]
    fn filter_only_empty_and_whitespace_queries_are_valid() {
        let raw = "kind:pdf modified:1w";
        assert_eq!(
            FileQuery::parse(raw).unwrap(),
            FileQuery {
                terms: Vec::new(),
                qualifier_ranges: vec![0..8, 9..20],
                kind: Some(FileKind::Pdf),
                extension: None,
                folder: None,
                modified_within: Some(Duration::from_secs(7 * 24 * 60 * 60)),
            }
        );
        assert_eq!(FileQuery::parse("").unwrap(), FileQuery::default());
        assert_eq!(FileQuery::parse(" \t\n ").unwrap(), FileQuery::default());
    }

    #[test]
    fn error_messages_never_echo_query_values() {
        let error = FileQuery::parse("kind:private-value").unwrap_err();
        assert_eq!(
            error.message(),
            "Use kind:pdf, image, text, audio, video, folder, archive, or app."
        );
        assert!(!error.message().contains("private-value"));
    }
}
