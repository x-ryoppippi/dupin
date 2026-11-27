use std/assert
use ../search-quality.nu [
    against
    checked-results
    hardest
    metrics
    ranks-text
    render
    summary
    summary-line
]

def case [id: string, query: string, ranks: list<any>]: nothing -> record {
    {
        id: $id
        intent: "app:Ardia"
        query: $query
        history: ["base"]
        tags: ["prefix"]
        ranks: $ranks
    }
}

def results [cases: list<record>]: nothing -> record {
    {
        schema: 1
        commit: ("a" | fill --character a --width 40)
        fixtures: ("f" | fill --character f --width 64)
        cases: $cases
    }
}

def main [] {

    # Keystrokes until first, the cost of a case never first, and dropping out of the 8 rows.
    let first = case one "ard" [5 2 1] | metrics
    assert equal [$first.keystrokes $first.cost $first.final_rank $first.dropped_out] [3 3 1 false]
    let never = case two "ar" [2 null] | metrics
    assert equal [$never.keystrokes $never.cost $never.dropped_out] [null 3 true]
    assert equal (case three "abc" [9 12 3] | metrics | get dropped_out) false
    assert equal (case four "abc" [3 9 1] | metrics | get dropped_out) true
    assert equal ([1 null 12] | ranks-text) "1·–·12"

    let figures = summary [
        $first
        $never
        (case five "a" [1] | metrics)
    ]
    assert equal $figures.cases 3
    assert equal $figures.mean_keystrokes ((3 + 3 + 1) / 3)
    assert equal ($figures.within | get cases) [1 1 2 2]
    assert equal $figures.never_first 1
    assert equal (hardest [$first $never] | get id) [two one]
    assert (
        summary-line $figures
        | str ends-with "never first in 1 of 3 cases; 1 drops out of the visible rows."
    )

    # The report reads a pull request's results with a token that can comment.
    let good = results [
        (case one "ard" [5 2 1])
    ]
    assert equal (($good | checked-results).cases.0.keystrokes) 3
    assert error { results [(case one "ard" [5 2])] | checked-results }
    assert error { results [(case one "ard" [5 2 0])] | checked-results }
    assert error { results [(case one "ard" [5 2 "1"])] | checked-results }
    assert error { results [(case "One" "ard" [5 2 1])] | checked-results }
    assert error { results [(case one "a`b" [1 1 1])] | checked-results }
    assert error { results [(case one "a|b" [1 1 1])] | checked-results }
    assert error { results [(case one "ard" [5 2 1] | update intent "app:[x](https://example.com)")] | checked-results }
    assert error { results [(case one "ard" [5 2 1] | update tags ["<b>"])] | checked-results }
    assert error { results [(case one "ard" [5 2 1]) (case one "ard" [5 2 1])] | checked-results }
    assert error { $good | update commit "main; rm -rf /" | checked-results }
    assert error { $good | update fixtures "x" | checked-results }

    # More keystrokes or a lower rank after any keystroke regresses; fewer, with none lower, improves.
    let main = [
        (case same "ard" [5 2 1])
        (case slower "ard" [5 2 1])
        (case lower "ard" [3 2 1])
        (case faster "ard" [5 2 1])
        (case edited "ard" [5 2 1])
        (case gone "ard" [5 2 1])
    ] | each { metrics }
    let pr = [
        (case same "ard" [5 2 1])
        (case slower "ard" [5 2 2])
        (case lower "ard" [4 1 1])
        (case faster "ard" [5 1 1])
        (case edited "ardia" [5 2 1 1 1])
        (case added "ard" [1 1 1])
    ] | each { metrics }
    let compared = against $pr $main
    assert equal ($compared | select id verdict) [
        [id verdict];
        [same same]
        [slower regressed]
        [lower regressed]
        [faster improved]
        [edited changed]
        [added new]
        [gone removed]
    ]

    let measured = results $pr | checked-results
    let base = results $main | checked-results
    let found = {
        data: $base
        id: 7
        commit: ("b" | fill --character b --width 40)
        url: "https://example.com/run"
        merge_base: true
        base: ("b" | fill --character b --width 40)
    }
    let comment = render $measured $found {commit: ("c" | fill --character c --width 40), run_url: "https://example.com/pr"}
    let lines = $comment | lines
    assert equal $lines.0 "<!-- dupin-search-quality -->"
    assert equal $lines.1 "### Search quality on ccccccc"
    assert (
        $comment
        | str contains "Against main at [bbbbbbb](https://example.com/run), which this pull request merged onto."
    )
    assert (
        $comment
        | str contains ":warning: **2 cases need more keystrokes or rank lower than on main: `slower`, `lower`.**"
    )
    assert (
        $comment
        | str contains "| `slower` | `ard` | `app:Ardia` | 5·2·1 | 5·2·2 | 3 → never |"
    )
    assert ($comment | str contains "#### What to do")
    let machine = $comment | parse --regex '(?s)```json\n(?<json>.*)\n```' | get 0.json | from json
    assert equal ($machine.regressed | get id) [slower lower]
    assert equal $machine.improved [faster]
    assert equal $machine.removed [gone]
    assert equal $machine.fixtures_changed false

    let calm = render (results [
        (case same "ard" [5 2 1])
    ] | checked-results) $found {commit: "", run_url: ""}
    assert (
        $calm
        | str contains "No case needs more keystrokes or ranks lower than on main."
    )
    assert not ($calm | str contains "What to do")
    let alone = render $measured null {commit: "", run_url: ""}
    assert ($alone | str contains "No main build has measured search quality yet")
}
