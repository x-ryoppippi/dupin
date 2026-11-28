use std/assert
use ../nightly.nu [published-commit decide]

const HEAD = "912be04539fb0590f9feaa7fa2a4ee0af5ebfafd"

def feed [version]: nothing -> string {
    {
        version: $version
        build: 417
        url: "https://example.invalid/Dupin.zip"
        sha256: "00"
    } | to json
}

def main [] {

    # The feed names the abbreviated commit it was built from.
    assert equal (published-commit (feed "0.1.0-nightly.417+912be04")) {commit: "912be04", problem: null}
    assert equal (published-commit (feed $"0.1.0-nightly.417+($HEAD)")) {commit: $HEAD, problem: null}

    # An unreadable feed is a problem, never a match.
    assert ((published-commit "not json").problem != null)
    assert ((published-commit "[]").problem != null)
    assert ((published-commit (feed null)).problem != null)
    assert ((published-commit (feed 417)).problem != null)
    assert ((published-commit (feed "0.1.0")).problem != null)
    assert ((published-commit (feed "0.1.0-nightly.417+")).problem != null)
    assert ((published-commit (feed "0.1.0-nightly.417+912b")).problem != null)

    # The same commit skips, by prefix for an abbreviated one.
    assert equal (decide $HEAD {commit: "912be04", problem: null}).build false
    assert equal (decide $HEAD {commit: $HEAD, problem: null}).build false
    assert equal (decide $HEAD {commit: "912be04", problem: null}).level notice

    # A different commit, no nightly yet, or an unreadable feed builds.
    assert equal (decide $HEAD {commit: "140998e", problem: null}).build true
    assert equal (decide $HEAD {commit: null, problem: null}) {
        build: true
        level: notice
        message: $"No nightly is published yet; building ($HEAD)"
    }
    let unreadable = decide $HEAD {commit: null, problem: "update.json has no version"}
    assert equal $unreadable.build true
    assert equal $unreadable.level warning
}
