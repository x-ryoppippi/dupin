//! `dupin-search-quality --corpus <corpus.toml> --cases <cases.toml> [--out <results.json>]`:
//! where the row each case wants is after every character of its query, as JSON.
//! `scripts/search-quality.nu` runs it and reads the results.

mod bench;
mod fixture;

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::bench::Bench;
use crate::fixture::{Cases, Corpus};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dupin-search-quality: {error}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str =
    "usage: dupin-search-quality --corpus <corpus.toml> --cases <cases.toml> [--out <results.json>]";

fn run(args: Vec<String>) -> Result<(), String> {
    let mut corpus = None;
    let mut cases = None;
    let mut out = None;
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let slot = match flag.as_str() {
            "--corpus" => &mut corpus,
            "--cases" => &mut cases,
            "--out" => &mut out,
            _ => return Err(format!("unknown argument {flag}\n{USAGE}")),
        };
        *slot = Some(PathBuf::from(
            args.next().ok_or_else(|| format!("{flag} needs a path"))?,
        ));
    }
    let read = |path: Option<PathBuf>| {
        let path = path.ok_or_else(|| USAGE.to_owned())?;
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))
    };
    let results = measure(&read(corpus)?, &read(cases)?)?;
    let text = serde_json::to_string_pretty(&results).map_err(|error| error.to_string())? + "\n";
    match out {
        Some(path) => std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display())),
        None => {
            print!("{text}");
            Ok(())
        }
    }
}

/// Every case of `cases` over `corpus`, with the rank of its row after each keystroke.
fn measure(corpus: &str, cases: &str) -> Result<serde_json::Value, String> {
    let bench = Bench::new(Corpus::parse(corpus)?, dupin_mac::kinds::Rows::native())?;
    let measured = Cases::parse(cases)?
        .cases
        .into_iter()
        .map(|case| {
            let ranks = bench.ranks(&case)?;
            let mut row = serde_json::to_value(&case).map_err(|error| error.to_string())?;
            row["ranks"] = json!(ranks);
            Ok(row)
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({"schema": 1, "cases": measured}))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORPUS: &str = r#"
        [apps]
        "/System/Applications" = ["Music", "Calendar", "FaceTime", "Maps"]
        "/Applications" = ["Arc", "Ardia", "Arto", "Spark", "Visual Studio Code"]

        [names]
        Music = ["Apple Music", "Apple Radio"]

        [[commands]]
        id = "calendar.events"
        title = "Calendar"
        keywords = ["cal"]

        [web]
        g = "Google"
        maps = "Apple Maps"

        [aliases]
        vs = "Visual Studio Code"

        [[history.base]]
        app = "Arto"
        times = 50

        [[history.wrong-picks]]
        app = "Spark"
        query = "ar"
        times = 10
    "#;

    fn ranks(cases: &str) -> Vec<Vec<Option<usize>>> {
        let measured = measure(CORPUS, cases).unwrap();
        measured["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| serde_json::from_value(case["ranks"].clone()).unwrap())
            .collect()
    }

    #[test]
    fn ranks_the_wanted_row_after_every_keystroke() {
        let measured = ranks(
            r#"
            [[cases]]
            id = "ardia"
            intent = "app:Ardia"
            query = "ardia"
            history = ["base", "wrong-picks"]

            [[cases]]
            id = "time"
            intent = "app:FaceTime"
            query = "time"
            history = []
            "#,
        );
        assert_eq!(
            measured[0][1..],
            [Some(3), Some(1), Some(1), Some(1)],
            "frecent Arto and shorter Arc lead `ar`, and Spark's picks for it don't lift it over them"
        );
        assert_eq!(measured[1][3], Some(1));
    }

    #[test]
    fn keywords_web_searches_and_aliases_are_rows_too() {
        let measured = ranks(
            r#"
            [[cases]]
            id = "cal"
            intent = "command:calendar.events"
            query = "cal"

            [[cases]]
            id = "maps"
            intent = "web:maps"
            query = "maps"

            [[cases]]
            id = "vs"
            intent = "app:Visual Studio Code"
            query = "vs"

            [[cases]]
            id = "radio"
            intent = "app:Music"
            query = "radio"
            "#,
        );
        assert_eq!(measured[0][2], Some(1), "an exact keyword of three letters leads");
        assert_eq!(measured[1][3], Some(1), "a web search's alias is pinned first");
        assert_eq!(measured[2][1], Some(1), "an alias leads");
        assert_eq!(measured[3][4], Some(1), "an alternate name finds the app");
    }

    #[test]
    fn a_case_the_corpus_cannot_answer_fails_the_run() {
        for case in [
            r#"intent = "app:Nowhere""#,
            r#"intent = "command:nothing.here""#,
            r#"intent = "web:nope""#,
            r#"intent = "Ardia""#,
            "intent = \"app:Ardia\"\nhistory = [\"unknown\"]",
        ] {
            let cases = format!("[[cases]]\nid = \"x\"\nquery = \"a\"\n{case}\n");
            assert!(measure(CORPUS, &cases).is_err(), "{case}");
        }
        let twice = "[[cases]]\nid = \"x\"\nintent = \"app:Arc\"\nquery = \"a\"\n".repeat(2);
        assert!(measure(CORPUS, &twice).is_err());
    }

    /// The cases CI measures run without error, and the scenario of #300 stays fixed.
    #[test]
    fn the_repository_cases_run() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/search-quality");
        let read = |name: &str| std::fs::read_to_string(format!("{root}/{name}")).unwrap();
        let measured = measure(&read("corpus.toml"), &read("cases.toml")).unwrap();
        let cases = measured["cases"].as_array().unwrap();
        assert!(cases.len() >= 100, "{} cases", cases.len());
        let ardia = cases.iter().find(|case| case["id"] == "issue-300.ardia").unwrap();
        let ranks: Vec<Option<usize>> = serde_json::from_value(ardia["ranks"].clone()).unwrap();
        assert!(
            ranks[1..].iter().all(|rank| rank.is_some_and(|rank| rank <= 8)),
            "{ranks:?}"
        );
        assert_eq!(ranks[2..], [Some(1), Some(1), Some(1)]);
    }
}
