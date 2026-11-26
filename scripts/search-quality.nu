#!/usr/bin/env nu

# Search quality: the distance from intent to result, measured in keystrokes.
#
# `run` types each case in scripts/search-quality/cases.toml into the main search, one character
# at a time, over the fixture apps, keywords and history in scripts/search-quality/corpus.toml,
# with the `dupin-search-quality` benchmark, and writes where the wanted row was after every
# keystroke as JSON. `compare` holds one result set against another, and `report` renders a pull
# request's against the main build it merged onto (scripts/main-build.nu) as the Markdown of the
# Search quality comment workflow's comment. Neither fails on a regression. See
# docs/development.md, "Search quality".

use main-build.nu find-main-artifact

def main []: nothing -> nothing {
    help main
}

# The rows the panel shows without scrolling: VISIBLE_ROWS in crates/dupin/src/view/results.rs.
export const VISIBLE_ROWS = 8
# The keystrokes the summary gives the share of cases first within.
export const WITHIN = [1 2 3 5]
const MARKER = "<!-- dupin-search-quality -->"
const CASE = '^[a-z0-9][a-z0-9.-]{0,63}$'
const NAME = '^[a-z0-9][a-z0-9-]{0,31}$'
const INTENT = '^(app|command|web|system):[A-Za-z0-9 .&+-]{1,64}$'
const QUERY = '^[\p{L}\p{N} .?&+-]{1,40}$'
const SHA = '^[0-9a-f]{40}$'
const MOST_RANK = 1000

# Measure every case and write the results as JSON.
def "main run" [
    --bin: path = "target/debug/dupin-search-quality" # `cargo build -p dupin-search-quality`
    --corpus: path = "scripts/search-quality/corpus.toml"
    --cases: path = "scripts/search-quality/cases.toml"
    --commit: string # The commit measured; HEAD by default
    --out: path = "target/search-quality/results.json"
]: nothing -> nothing {
    cd ($env.FILE_PWD | path dirname)
    let measured = ^$bin --corpus $corpus --cases $cases | complete
    if $measured.exit_code != 0 {
        error make {msg: $"the benchmark failed: ($measured.stderr | str trim)"}
    }
    let commit = $commit | default { ^git rev-parse HEAD | str trim }
    let report = {
        schema: 1
        created: (date now | format date "%+")
        commit: $commit
        fixtures: (fixtures $corpus $cases)
        cases: ($measured.stdout | from json | get cases)
    }
    mkdir ($out | path dirname)
    $report | to json | save --force $out
    print --stderr $"wrote ($out)"
    let checked = $report | checked-results
    print (summary-line (summary $checked.cases))
    print (
        hardest $checked.cases
        | select id query ranks
        | update ranks { ranks-text }
        | table
    )
}

# Hold `candidate`'s results against `baseline`'s, case by case. Prints every case that
# changed; never fails on one.
def "main compare" [baseline: path, candidate: path]: nothing -> nothing {
    let base = open $baseline | checked-results
    let cand = open $candidate | checked-results
    let compared = against $cand.cases $base.cases
    print $"baseline:  (summary-line (summary $base.cases))"
    print $"candidate: (summary-line (summary $cand.cases))"
    print (
        $compared
        | where verdict != same
        | select id verdict main_ranks pr_ranks main_keystrokes pr_keystrokes
        | update main_ranks { ranks-text }
        | update pr_ranks { ranks-text }
        | table
    )
}

# Render the results in `results` against main's as Markdown, the Search quality comment
# workflow's comment. With `--repo`, main's are those of the CI run on the main commit the
# results' commit merged onto, or else the latest main run that measured any.
def "main report" [
    results: path # `run`'s output
    --repo: string # The GitHub repository whose main build to compare with
    --branch: string = "main"
    --commit: string # The pull request's head, to name in the heading
    --run-url: string # The CI run that measured `results`
]: nothing -> string {
    let measured = open $results | checked-results
    let main = if $repo == null { null } else {
        find-main-artifact $repo $measured.commit $branch search-quality results.json {|file|
            open $file | checked-results
        }
    }
    render $measured $main {
        commit: ($commit | default $measured.commit)
        run_url: $run_url
    }
}

# A fingerprint of the corpus and cases, so a report can tell a change to them from a change to
# ranking.
def fixtures [corpus: path, cases: path]: nothing -> string {
    [$corpus $cases] | each { open --raw | hash sha256 } | str join | hash sha256
}

# `run`'s JSON, holding only what `compare` and `report` read, each field checked. The report
# reads a pull request's results with a token that can comment, so every text it prints must be
# one a case could have and every rank a number.
export def checked-results []: any -> record {
    let results = $in
    if not ($results | describe | str starts-with record) {
        error make {msg: "the results are not a record"}
    }
    let text = {|value, pattern: string, what: string|
        if ($value | describe) != string or not ($value =~ $pattern) {
            error make {msg: $"($what) is not valid"}
        }
        $value
    }
    let names = {|value, what: string|
        if not (($value | describe) =~ '^(list|table)') {
            error make {msg: $"($what) is not a list"}
        }
        $value | each {|name| do $text $name $NAME $what }
    }
    let commit = $results | get --optional commit | default ""
    if $commit != "" { do $text $commit $SHA "the commit" }
    let cases = $results | get --optional cases
    if not (($cases | describe) =~ '^(list|table)') {
        error make {msg: "the cases are not a list"}
    }
    let checked = $cases | each {|case|
        if not ($case | describe | str starts-with record) {
            error make {msg: "a case is not a record"}
        }
        let id = do $text ($case | get --optional id) $CASE "a case's id"
        let query = do $text ($case | get --optional query) $QUERY $"($id)'s query"
        let ranks = $case | get --optional ranks
        if not (($ranks | describe) =~ '^(list|table)') or ($ranks | length) != ($query | split chars | length) {
            error make {msg: $"($id)'s ranks are not one per character of its query"}
        }
        let ranks = $ranks | each {|rank|
            if $rank != null and (($rank | describe) != int or $rank < 1 or $rank > $MOST_RANK) {
                error make {msg: $"($id)'s ranks are not ranks"}
            }
            $rank
        }
        {
            id: $id
            intent: (do $text ($case | get --optional intent) $INTENT $"($id)'s intent")
            query: $query
            history: (do $names ($case | get --optional history | default []) $"($id)'s history")
            tags: (do $names ($case | get --optional tags | default []) $"($id)'s tags")
            ranks: $ranks
        }
    }
    if ($checked | get id | uniq | length) != ($checked | length) {
        error make {msg: "a case is listed twice"}
    }
    {
        commit: $commit
        fixtures: (
            (do
                $text
                ($results | get --optional fixtures)
                '^[0-9a-f]{64}$'
                "the fixtures' fingerprint"
            )
        )
        cases: ($checked | each { metrics })
    }
}

# A case with what its ranks say: `typed`, the characters in its query; `keystrokes`, how many
# of them put the wanted row first (none when it never is); `cost`, the same counting a case
# never first as its whole query and one more; `final_rank`; and `dropped_out`, whether the row
# left the visible rows after it was in them.
export def metrics []: record -> record {
    let case = $in
    let ranks = $case.ranks
    let visible = $ranks | each {|rank| $rank != null and $rank <= $VISIBLE_ROWS }
    let first = $ranks | enumerate | where item == 1 | get 0?.index
    let shown = $visible | enumerate | where item | get 0?.index
    let typed = $ranks | length
    let keystrokes = if $first == null { null } else { $first + 1 }
    $case | merge {
        typed: $typed
        keystrokes: $keystrokes
        cost: ($keystrokes | default ($typed + 1))
        final_rank: ($ranks | last)
        dropped_out: ($shown != null and ($visible | skip $shown | any {|seen| not $seen }))
    }
}

# The figures over `cases` (`metrics` rows): how many, the mean `cost`, how many are first within
# each of WITHIN keystrokes, how many never are, and how many drop out of the visible rows.
export def summary [cases: list<record>]: nothing -> record {
    let count = $cases | length
    let within = $WITHIN | each {|keys|
        let first = $cases | where {|case| $case.keystrokes != null and $case.keystrokes <= $keys } | length
        {keys: $keys, cases: $first, percent: (if $count == 0 { 0.0 } else { $first * 100.0 / $count })}
    }
    {
        cases: $count
        mean_keystrokes: (
            if $count == 0 {
                0.0
            } else {
                $cases | get cost | math avg
            }
        )
        within: $within
        never_first: ($cases | where keystrokes == null | length)
        dropped_out: ($cases | where dropped_out | length)
    }
}

# The cases that take the most keystrokes, those never first before the rest.
export def hardest [cases: list<record>, count: int = 10]: nothing -> list<record> {
    $cases
    | sort-by --custom {|a, b|
        let never = [($a.keystrokes == null) ($b.keystrokes == null)]
        if $never.0 != $never.1 { $never.0 } else if $a.cost != $b.cost { $a.cost > $b.cost } else { $a.id < $b.id }
    }
    | where {|case| $case.cost > 1 }
    | first $count
}

def rank-value [rank]: nothing -> int {
    $rank | default ($MOST_RANK + 1)
}

# Each case of `ours` against the case of the same id in `theirs`: `regressed` when it needs
# more keystrokes or its row is lower after any keystroke, `improved` when it needs fewer or is
# higher after one and lower after none, `same` otherwise; `new`, `changed` when its intent,
# query or history differ, and `removed` for a case only `theirs` has.
export def against [ours: list<record>, theirs: list<record>]: nothing -> list<record> {
    let row = {|case, base, verdict|
        {
            id: $case.id
            intent: $case.intent
            query: $case.query
            verdict: $verdict
            main_ranks: ($base | get --optional ranks)
            pr_ranks: ($case | get --optional ranks)
            main_keystrokes: ($base | get --optional keystrokes)
            pr_keystrokes: ($case | get --optional keystrokes)
        }
    }
    let compared = $ours | each {|case|
        let base = $theirs | where id == $case.id | get 0?
        if $base == null {
            do $row $case null new
        } else if [$base.intent $base.query $base.history] != [$case.intent $case.query $case.history] {
            do $row $case $base changed
        } else {
            let pairs = $case.ranks | zip $base.ranks | each {|pair|
                (rank-value $pair.0) - (rank-value $pair.1)
            }
            let verdict = if $case.cost > $base.cost or ($pairs | any {|delta| $delta > 0 }) {
                "regressed"
            } else if $case.cost < $base.cost or ($pairs | any {|delta| $delta < 0 }) {
                "improved"
            } else {
                "same"
            }
            do $row $case $base $verdict
        }
    }
    let removed = $theirs
    | where {|base| $base.id not-in ($ours | get id) }
    | each {|base| do $row ($base | reject ranks keystrokes) $base removed }
    $compared ++ $removed
}

export def ranks-text []: any -> string {
    if $in == null { "–" } else {
        $in | each {|rank| if $rank == null { "–" } else { $rank | into string } } | str join "·"
    }
}

def fixed [value: number, decimals: int]: nothing -> string {
    $value | into float | math round --precision $decimals | into string --decimals $decimals
}

def keys-text [keys]: nothing -> string {
    if $keys == null {
        "never"
    } else {
        $keys | into string
    }
}

export def summary-line [figures: record]: nothing -> string {
    let within = $figures.within
    | each {|share| $"($share.keys) key(if $share.keys == 1 { '' } else { 's' }) (fixed $share.percent 0)%" }
    | str join ", "
    $"(fixed $figures.mean_keystrokes 2) keystrokes on average until the wanted row is first; first within ($within); never first in ($figures.never_first) of ($figures.cases) cases; ($figures.dropped_out) (if $figures.dropped_out == 1 { 'drops' } else { 'drop' }) out of the visible rows."
}

# The comment's Markdown. `main` is `find-main-artifact`'s record with main's results as `data`,
# or nothing.
export def render [measured: record, main, context: record]: nothing -> string {
    let short = {|commit|
        $commit | str substring 0..6
    }
    let ours = summary $measured.cases
    let heading = if ($context.commit? | default "") == "" { "### Search quality" } else {
        $"### Search quality on (do $short $context.commit)"
    }
    let main_link = if $main == null { "main" } else { $"main at [(do $short $main.commit)]\(($main.url)\)" }
    let against_text = if $main == null {
        "No main build has measured search quality yet, so there is nothing to compare with."
    } else if $main.merge_base {
        $"Against ($main_link), which this pull request merged onto."
    } else if $main.base != "" {
        $"Against ($main_link), the latest main build measured; main at (do $short $main.base), which this pull request merged onto, has no measurement."
    } else {
        $"Against ($main_link), the latest main build measured."
    }
    let compared = if $main == null { [] } else { against $measured.cases $main.data.cases }
    let fixtures_changed = $main != null and $main.data.fixtures != $measured.fixtures
    let theirs = if $main == null { null } else { summary $main.data.cases }
    let headline = if $theirs == null {
        $"**(fixed $ours.mean_keystrokes 2) keystrokes** on average until the wanted row is first."
    } else {
        let delta = $ours.mean_keystrokes - $theirs.mean_keystrokes
        let sign = if $delta > 0 { "+" } else if $delta < 0 { "-" } else { "±" }
        $"**(fixed $ours.mean_keystrokes 2) keystrokes** on average until the wanted row is first \(main: (fixed $theirs.mean_keystrokes 2), ($sign)(fixed ($delta | math abs) 2)\)."
    }
    let figures = [
        $headline
        $"first within ($ours.within | get keys | str join ' / ') keys: ($ours.within | each {|share| $'(fixed $share.percent 0)%' } | str join ' / ')"
        $"never first: ($ours.never_first) of ($ours.cases) cases · drop out of the top ($VISIBLE_ROWS): ($ours.dropped_out)"
    ] | str join "<br>\n"
    let fixtures_note = if $fixtures_changed {
        " This pull request changes the benchmark's corpus or cases, so a difference may come from them rather than from ranking."
    } else { "" }
    let regressed = $compared | where verdict == regressed
    let improved = $compared | where verdict == improved
    let changes = if ($regressed | is-empty) {
        if $main == null { [] } else {
            let better = if ($improved | is-empty) { "" } else {
                $" ($improved | length) improved: ($improved | each {|row| $"`($row.id)`" } | str join ', ')."
            }
            [
                $"No case needs more keystrokes or ranks lower than on main.($better)"
            ]
        }
    } else {
        let count = $regressed | length
        let names = $regressed | each {|row| $"`($row.id)`" } | str join ", "
        let better = if ($improved | is-empty) { "" } else { $" ($improved | length) improved." }
        [
            $":warning: **($count) case(if $count == 1 { '' } else { 's' }) need(if $count == 1 { 's' } else { '' }) more keystrokes or rank(if $count == 1 { 's' } else { '' }) lower than on main: ($names).**($better) Search quality never blocks merging; this comment is for whoever works on or reviews this pull request."
            ([
                "| Case | Query | Wants | Ranks on main | Ranks here | First after |"
                "| :-- | :-- | :-- | :-- | :-- | :-- |"
                ...($regressed | each {|row|
                    $"| `($row.id)` | `($row.query)` | `($row.intent)` | ($row.main_ranks | ranks-text) | ($row.pr_ranks | ranks-text) | (keys-text $row.main_keystrokes) → (keys-text $row.pr_keystrokes) |"
                })
            ] | str join "\n")
            (what-to-do)
        ]
    }
    let run = if ($context.run_url? | default "") == "" { "" } else { $" Measured by [this run]\(($context.run_url)\)." }
    let verdicts = $compared | reduce --fold {} {|row, all| $all | insert $row.id $row.verdict }
    let every = [
        "<details><summary>Every case</summary>\n"
        $"Each case types its query one character at a time into the main search over `scripts/search-quality/corpus.toml`; the ranks are where the wanted row is after each character \(– when it is not among the results\), and a case never first counts as its whole query and one more in the mean.($run)\n"
        "| Case | Query | Wants | Ranks | First after | |"
        "| :-- | :-- | :-- | :-- | --: | :-- |"
        ...($measured.cases | each {|case|
            let mark = match ($verdicts | get --optional $case.id) {
                regressed => ":warning: worse"
                improved => "better"
                new => "new"
                changed => "changed"
                _ => ""
            }
            $"| `($case.id)` | `($case.query)` | `($case.intent)` | ($case.ranks | ranks-text) | (keys-text $case.keystrokes) | ($mark) |"
        })
        "\n</details>"
    ] | str join "\n"
    let hardest_cases = [
        "<details><summary>Hardest cases</summary>\n"
        "| Case | Query | Wants | Ranks | First after |"
        "| :-- | :-- | :-- | :-- | --: |"
        ...(hardest $measured.cases | each {|case|
            $"| `($case.id)` | `($case.query)` | `($case.intent)` | ($case.ranks | ranks-text) | (keys-text $case.keystrokes) |"
        })
        "\n</details>"
    ] | str join "\n"
    let machine = {
        schema: 1
        commit: ($context.commit? | default $measured.commit)
        main: (
            if $main == null {
                null
            } else {
                {
                    commit: $main.commit
                    run_id: $main.id
                    merge_base: $main.merge_base
                }
            }
        )
        fixtures_changed: $fixtures_changed
        visible_rows: $VISIBLE_ROWS
        summary: (machine-figures $ours)
        main_summary: (if $theirs == null { null } else { machine-figures $theirs })
        regressed: (
            $regressed
            | select id intent query main_ranks pr_ranks main_keystrokes pr_keystrokes
        )
        improved: ($improved | get id)
        new: ($compared | where verdict == new | get id)
        changed: ($compared | where verdict == changed | get id)
        removed: ($compared | where verdict == removed | get id)
    }
    let summary_block = $"<details><summary>Machine-readable summary</summary>\n\n```json\n($machine | to json --indent 2)\n```\n\n</details>"
    [
        $"($MARKER)\n($heading)"
        $figures
        $"($against_text)($fixtures_note)"
        ...$changes
        $hardest_cases
        $every
        $summary_block
    ]
    | str join "\n\n"
    | $"($in)\n"
}

def machine-figures [figures: record]: nothing -> record {
    $figures
    | update mean_keystrokes { math round --precision 3 }
    | update within { each {|share| {keys: $share.keys, cases: $share.cases} } }
}

def what-to-do []: nothing -> string {
    [
        "#### What to do"
        ""
        "1. Look at what this pull request changes in ranking: the matcher (`crates/dupin-core/src/fuzzy.rs`), the history's boosts (`crates/dupin-core/src/history.rs`), the apps source (`crates/dupin-mac/src/apps.rs`), the rows that name keywords (`crates/dupin-core/src/session/keywords.rs`) and how providers' rows are merged (`crates/dupin-core/src/session/search.rs`)."
        "2. Reproduce it: `cargo build -p dupin-search-quality` and `nu scripts/search-quality.nu run` on this branch and on main (with `--out`), then `nu scripts/search-quality.nu compare <main's> <this branch's>`. The benchmark is deterministic, so a regression here is a real one."
        "3. If the trade is intended, say why in the pull request. If a case no longer asks for what a person would want, change it in `scripts/search-quality/cases.toml` in this pull request and say why; keep its id when the intent stays the same."
    ] | str join "\n"
}
