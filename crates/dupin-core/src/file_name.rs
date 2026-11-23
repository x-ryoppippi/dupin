//! File-name rules for operations that create or rename entries: which names are valid, and
//! which names to try, in order, so a new entry never replaces an existing one.

/// The longest name, in UTF-16 code units, that APFS and HFS+ volumes accept.
const NAME_MAX: usize = 255;

/// Why a proposed name can't be used.
pub fn problem(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        Some("Enter a name.")
    } else if name == "." || name == ".." {
        Some("This name is reserved.")
    } else if name.contains('/') || name.contains(':') {
        // POSIX names can't hold "/"; Finder shows a stored ":" as "/", so it is refused too.
        Some("A name can't contain “/” or “:”.")
    } else if name.contains('\0') {
        Some("A name can't contain a null character.")
    } else if name.encode_utf16().count() > NAME_MAX {
        Some("This name is too long.")
    } else {
        None
    }
}

/// The byte length of `name` before its extension: what Finder selects when renaming.
/// `extension` is false for folders, whose dots belong to the name.
pub(crate) fn stem_len(name: &str, extension: bool) -> usize {
    split(name, extension).0.len()
}

/// Names for a duplicate of `name`, best first: "report copy.pdf", "report copy 2.pdf", …
/// Duplicating a copy continues its numbering instead of stacking another "copy".
pub fn duplicate_names(name: &str, extension: bool) -> impl Iterator<Item = String> + use<> {
    let (stem, ext) = split(name, extension);
    let base = format!("{} copy", strip_copy(stem));
    numbered(base, ext.to_owned())
}

/// Names for a zip archive of `name`, best first: "report.pdf.zip", "report.pdf 2.zip", …
pub fn archive_names(name: &str) -> impl Iterator<Item = String> + use<> {
    numbered(name.to_owned(), ".zip".to_owned())
}

fn numbered(stem: String, ext: String) -> impl Iterator<Item = String> {
    std::iter::once(format!("{stem}{ext}")).chain((2u64..).map(move |n| format!("{stem} {n}{ext}")))
}

/// `name` as stem and extension (with its dot). A leading dot marks a hidden file, not an
/// extension, and an extension with spaces is part of the name ("Notes v1. final").
fn split(name: &str, extension: bool) -> (&str, &str) {
    if !extension {
        return (name, "");
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() && !name[dot + 1..].contains(' ') => name.split_at(dot),
        _ => (name, ""),
    }
}

/// "report copy" and "report copy 3" back to "report".
fn strip_copy(stem: &str) -> &str {
    let numbered = stem
        .rsplit_once(' ')
        .filter(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .map_or(stem, |(rest, _)| rest);
    match numbered.strip_suffix(" copy") {
        Some(base) if !base.is_empty() => base,
        _ => stem
            .strip_suffix(" copy")
            .filter(|b| !b.is_empty())
            .unwrap_or(stem),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(names: impl Iterator<Item = String>, n: usize) -> Vec<String> {
        names.take(n).collect()
    }

    #[test]
    fn duplicates_follow_finder_naming() {
        assert_eq!(
            first(duplicate_names("report.pdf", true), 3),
            ["report copy.pdf", "report copy 2.pdf", "report copy 3.pdf"]
        );
        assert_eq!(
            first(duplicate_names("Docs", false), 2),
            ["Docs copy", "Docs copy 2"]
        );
        assert_eq!(first(duplicate_names("v1.2", false), 1), ["v1.2 copy"]);
        assert_eq!(
            first(duplicate_names("Example.app", true), 1),
            ["Example copy.app"]
        );
        assert_eq!(
            first(duplicate_names("archive.tar.gz", true), 1),
            ["archive.tar copy.gz"]
        );
        assert_eq!(first(duplicate_names(".zshrc", true), 1), [".zshrc copy"]);
        assert_eq!(first(duplicate_names("trailing.", true), 1), ["trailing. copy"]);
        assert_eq!(
            first(duplicate_names("Notes v1. final", true), 1),
            ["Notes v1. final copy"]
        );
    }

    #[test]
    fn duplicating_a_copy_continues_its_numbering() {
        assert_eq!(
            first(duplicate_names("report copy.pdf", true), 1),
            ["report copy.pdf"]
        );
        assert_eq!(
            first(duplicate_names("report copy 4.pdf", true), 2),
            ["report copy.pdf", "report copy 2.pdf"]
        );
        assert_eq!(first(duplicate_names("copy", false), 1), ["copy copy"]);
        assert_eq!(first(duplicate_names(" copy", false), 1), [" copy copy"]);
        assert_eq!(first(duplicate_names("Track 2", false), 1), ["Track 2 copy"]);
    }

    #[test]
    fn archives_keep_the_full_name_and_number_before_the_extension() {
        assert_eq!(
            first(archive_names("report.pdf"), 3),
            ["report.pdf.zip", "report.pdf 2.zip", "report.pdf 3.zip"]
        );
        assert_eq!(first(archive_names("Docs"), 2), ["Docs.zip", "Docs 2.zip"]);
    }

    #[test]
    fn the_first_free_candidate_never_replaces_an_existing_name() {
        let existing = ["report copy.pdf", "report copy 2.pdf"];
        let free = duplicate_names("report.pdf", true)
            .find(|name| !existing.contains(&name.as_str()))
            .unwrap();
        assert_eq!(free, "report copy 3.pdf");
    }

    #[test]
    fn rename_validation_refuses_names_the_file_system_or_finder_reject() {
        assert_eq!(problem("notes.txt"), None);
        assert_eq!(problem(".hidden"), None);
        assert_eq!(problem("日本語 ファイル"), None);
        assert!(problem("").is_some());
        assert!(problem("   ").is_some());
        assert!(problem(".").is_some());
        assert!(problem("..").is_some());
        assert!(problem("a/b").is_some());
        assert!(problem("a:b").is_some());
        assert!(problem("a\0b").is_some());
        assert_eq!(problem(&"a".repeat(255)), None);
        assert!(problem(&"a".repeat(256)).is_some());
        assert_eq!(problem(&"あ".repeat(255)), None);
        assert!(problem(&"あ".repeat(256)).is_some());
        assert_eq!(problem(&"😀".repeat(127)), None);
        assert!(problem(&"😀".repeat(128)).is_some());
    }

    #[test]
    fn the_stem_excludes_only_a_file_extension() {
        assert_eq!(stem_len("report.pdf", true), 6);
        assert_eq!(stem_len("report.pdf", false), 10);
        assert_eq!(stem_len(".zshrc", true), 6);
        assert_eq!(stem_len("写真.jpeg", true), "写真".len());
    }
}
