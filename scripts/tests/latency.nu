use std/assert
use ../latency.nu [
    COMPARE_FLOOR
    COMPARE_TOLERANCE
    CONTROL
    PARITY_FLOOR
    against-main
    checked-main
    checked-results
    display-groups
    render
    parity-pairs
    stale
    uncontrolled
    unheld
    unmeasured
    verdicts
]
use ../latency/measure.nu [
    enabled-grants
    fingerprint
    percentiles
    resolve
    typing-script
    unanswered
    unenabled
]

def result [id: string, implementation: string, p95: float]: nothing -> record {
    {id: $id, implementation: $implementation, p95_ms: $p95}
}

def verdict [baseline, candidate: record]: nothing -> string {
    verdicts [
        {baseline: $baseline, candidate: $candidate}
    ] 10.0 1ms | get 0.verdict
}

def main [] {

    # The tolerance is 10% or 1 ms, whichever is larger.
    assert equal (verdict (result a native 5.0) (result a native 5.9)) ok
    assert equal (verdict (result a native 5.0) (result a native 6.1)) regressed
    assert equal (verdict (result a native 20.0) (result a native 21.9)) ok
    assert equal (verdict (result a native 20.0) (result a native 22.1)) regressed
    assert equal (verdict (result a native 20.0) (result a extension 3.0)) ok
    assert equal (verdict null (result a extension 3.0)) new

    # Parity allows 10% or half a millisecond, which catches Dictionary's port before its call
    # overhead was cut: its p95 was 2.33 ms at best, against native's recorded 1.38 ms.
    let parity = {|baseline: record, candidate: record|
        verdicts [
            {baseline: $baseline, candidate: $candidate}
        ] 10.0 $PARITY_FLOOR | get 0.verdict
    }
    assert equal (do $parity (result a native 1.38) (result a extension 1.85)) ok
    assert equal (do $parity (result a native 1.38) (result a extension 2.33)) regressed
    assert equal (do $parity (result a native 20.0) (result a extension 21.9)) ok

    # A feature with no native implementation is held to its own budget instead.
    let budgeted = {|p95|
        result a extension $p95 | insert budget_ms 5.0
    }
    assert equal (verdict null (do $budgeted 4.9)) ok
    assert equal (verdict null (do $budgeted 5.1)) regressed

    # A port is held to its run's native implementation or, once that is deleted, to the one
    # recorded from the last commit that had it; a recorded scenario must still be measured.
    let recorded = [
        (
            {
                id: b
                feature: b
                commit: "b11a968d0"
                p95_ms: 1.5
            }
            | insert implementation native
        )
    ]
    let run = [
        (result a native 2.0)
        (result a extension 2.5)
        (result b extension 2.4)
        (result c extension 1.0)
    ]
    let pairs = parity-pairs $run $recorded
    assert equal ($pairs | each { $in.baseline?.implementation? | default none }) [native "native at b11a968" none]
    assert equal ($pairs | get candidate.id) [a b c]
    assert equal (verdicts $pairs 10.0 1ms | get verdict) [ok ok new]
    # `compare` may report that as new, but `parity` may not: c has no native result, no
    # recorded one and no budget, as a port whose native implementation was deleted without
    # recording its baseline would.
    assert equal (unheld $pairs) ["c [extension]"]
    assert equal (unheld (parity-pairs [
        (result c extension 1.0 | insert budget_ms 5.0)
    ] $recorded)) []
    assert equal (unheld (parity-pairs [
        (result b extension 2.4)
    ] $recorded)) []
    assert equal (unheld (parity-pairs [
        (result c native 1.0)
    ] $recorded)) []
    assert equal (
        parity-pairs [
            (result b native 1.0)
            (result b extension 2.4)
        ] $recorded
        | get 0.baseline.p95_ms
    ) 1.0
    assert equal (unmeasured $run $recorded) []

    # A recorded native result holds a port only when the same benchmark measured both: one
    # from another version of it, or from before results carried one, is stale.
    let settings = {
        samples: 210
        warmup: 10
        runs: 3
        gap_ms: 25.0
        fps: 1000
    }
    let scenario = {
        id: b
        feature: b
        prefix: "define "
        typed: "run"
    }
    let current = fingerprint $scenario $settings
    let held = {|recorded_benchmark, candidate_benchmark|
        let entry = $recorded | first | if $recorded_benchmark == null { $in } else { insert benchmark $recorded_benchmark }
        let port = result b extension 2.4 | if $candidate_benchmark == null { $in } else { insert benchmark $candidate_benchmark }
        stale (parity-pairs [$port] [$entry])
    }
    assert equal (do $held $current $current) []
    assert equal (do $held "earlier" $current) ["b [extension]"]
    assert equal (do $held null $current) ["b [extension]"]
    assert equal (do $held null null) ["b [extension]"]
    # The run's own native result needs no fingerprint, and neither does a port held to a budget.
    assert equal (stale (parity-pairs [
        (result b native 1.0)
        (result b extension 2.4)
    ] $recorded)) []
    assert equal (stale (parity-pairs [
        (result c extension 1.0 | insert budget_ms 5.0)
    ] $recorded)) []

    # The fingerprint follows what is typed and how the run measures it, not which
    # implementation answers.
    assert equal (fingerprint ($scenario | update id other | update feature other) $settings) $current
    assert not equal (fingerprint ($scenario | update typed "ran") $settings) $current
    assert not equal (fingerprint ($scenario | update prefix "def ") $settings) $current
    assert not equal (fingerprint $scenario ($settings | update samples 100)) $current
    assert not equal (fingerprint $scenario ($settings | update gap_ms 50.0)) $current
    assert equal (unmeasured [
        (result a extension 2.5)
    ] $recorded) [b]

    # A recorded baseline is scaled to this run's machine by the control recorded with it: the
    # median p95 of its scenarios this run, over theirs then. The tolerance applies to the scaled
    # figure, unchanged.
    let control = {
        scenarios: [
            {id: s1, benchmark: s1, p95_ms: 2.0}
            {id: s2, benchmark: s2, p95_ms: 2.2}
        ]
        p95_ms: 2.0
    }
    let controlled = $recorded | update 0.p95_ms 2.0 | insert 0.benchmark $current | insert 0.control $control
    let measured = {|s1: float, s2: float|
        [
            (result s1 native $s1 | insert benchmark s1)
            (result s2 native $s2 | insert benchmark s2)
        ]
    }
    let port = {|p95: float|
        result b extension $p95 | insert benchmark $current
    }
    let slow = parity-pairs ([
        (do $port 2.9)
    ] ++ (do $measured 2.4 2.6)) $controlled
    assert equal ($slow | get 0.control.scale) 1.25
    assert equal (stale $slow) []
    assert equal (uncontrolled $slow) []
    let row = verdicts $slow 10.0 $PARITY_FLOOR | first
    assert equal ($row | select baseline_p95_ms scale allowed_ms limit_ms verdict) {
        baseline_p95_ms: 2.0
        scale: 1.25
        allowed_ms: 0.5
        limit_ms: 3.0
        verdict: ok
    }
    # Unscaled, a machine a quarter slower would fail that port against a 2.5 ms limit; and on a
    # faster one, a port a third slower than native would pass it.
    assert equal (
        verdicts ($slow | each { reject control }) 10.0 $PARITY_FLOOR
        | get 0.verdict
    ) regressed
    let fast = parity-pairs ([
        (do $port 2.1)
    ] ++ (do $measured 1.5 1.5)) $controlled
    assert equal (verdicts $fast 10.0 $PARITY_FLOOR | get 0.limit_ms) 2.0
    assert equal (verdicts $fast 10.0 $PARITY_FLOOR | get 0.verdict) regressed
    # A pair held to the run's own native result is not scaled.
    let same_run = parity-pairs [
        (result b native 2.0)
        (do $port 2.4)
    ] $controlled
    assert equal ($same_run | get 0.control) null
    assert equal (uncontrolled $same_run) []
    assert equal (verdicts $same_run 10.0 $PARITY_FLOOR | get 0.scale) null
    # A recorded baseline without a control, or a run without one of its scenarios, can't be
    # scaled, so it fails rather than being held to another machine's figure.
    assert equal (
        uncontrolled (
            parity-pairs [
                (do $port 2.4)
            ] ($controlled | update 0 { reject control })
        )
    ) ["b [extension]"]
    assert equal (
        uncontrolled (
            parity-pairs [
                (do $port 2.4)
                (result s1 native 2.0 | insert benchmark s1)
            ] $controlled
        )
    ) ["b [extension]"]
    assert equal (uncontrolled (parity-pairs [
        (do $port 2.4)
    ] $controlled)) ["b [extension]"]
    # A control scenario measured by another benchmark than the recorded one is stale.
    let other_control = [
        (do $port 2.4)
        (result s1 native 2.0 | insert benchmark earlier)
        (result s2 native 2.0 | insert benchmark s2)
    ]
    assert equal (stale (parity-pairs $other_control $controlled)) ["b [extension]"]

    # `baseline` records the control from the same results, and refuses results without it or
    # without the feature, whose control would skew the recorded one.
    let tmp = mktemp --directory
    let report = {|results: list<record>|
        {schema: 1, commit: "abc1234", results: $results}
    }
    let stages = [dispatch_ms provider_ms merge_ms layout_ms] | each {|stage| {$stage: {p50: 0.1, p95: 0.2}} } | into record
    let native = {|id: string, feature: string, p95: float|
        {
            id: $id
            feature: $feature
            implementation: native
            benchmark: $id
            p50_ms: 1.0
            p95_ms: $p95
            p99_ms: 3.0
            stages: $stages
        }
    }
    let first = do $report [
        (do $native b b 1.5)
        (do $native s1 $CONTROL 2.0)
        (do $native s2 $CONTROL 2.4)
    ]
    let second = do $report [
        (do $native b b 1.7)
        (do $native s1 $CONTROL 2.2)
        (do $native s2 $CONTROL 2.6)
    ]
    $first | to json | save ($tmp | path join first.json)
    $second | to json | save ($tmp | path join second.json)
    let script = $env.FILE_PWD | path dirname | path join latency.nu
    let out = $tmp | path join baseline.json
    ^$nu.current-exe $script baseline b ($tmp | path join first.json) ($tmp | path join second.json) --out $out
    let entry = open $out | get results.0
    assert equal $entry.p95_ms 1.6
    assert equal $entry.control {
        scenarios: [
            {id: s1, benchmark: s1, p95_ms: 2.1}
            {id: s2, benchmark: s2, p95_ms: 2.5}
        ]
        p95_ms: 2.3
    }
    $first | update results { where feature != $CONTROL } | to json | save ($tmp | path join bare.json)
    let bare = do { ^$nu.current-exe $script baseline b ($tmp | path join bare.json) --out $out } | complete
    assert not equal $bare.exit_code 0
    $second | update results { where feature == $CONTROL } | to json | save ($tmp | path join controls.json)
    let partial = do {
        ^$nu.current-exe $script baseline b ($tmp | path join first.json) ($tmp | path join controls.json) --out $out
    } | complete
    assert not equal $partial.exit_code 0
    rm --recursive $tmp

    # Every keypress must search the feature's provider; only a pass's last must show its rows.
    let pass = [
        {
            searched: [apps.search web.keywords]
            shown: {apps.search: 3}
        }
        {
            searched: [apps.search web.keywords]
            shown: {web.keywords: 1, apps.search: 2}
        }
    ]
    assert equal (unanswered $pass web.keywords 2) 0
    assert equal (unanswered $pass apps.search 2) 0
    assert equal (unanswered ($pass | reverse) web.keywords 2) 1
    assert equal (unanswered $pass calendar.events 2) 2
    let typed_over = [
        {
            searched: []
            superseded: true
        }
        {
            searched: [web.keywords]
            shown: {web.keywords: 1}
        }
    ]
    assert equal (unanswered $typed_over web.keywords 2) 0

    # A port answers its built-in's own command, so only the package behind it tells the port
    # from the row the feature answers with while the package hasn't loaded.
    let ported = [
        {
            searched: [dictionary.lookup]
            packages: {dictionary.lookup: "bundled:dictionary"}
            shown: {dictionary.lookup: 1}
        }
    ]
    let stand_in = $ported | each { reject packages }
    assert equal (unanswered $ported dictionary.lookup 1 --package bundled:dictionary) 0
    assert equal (unanswered $stand_in dictionary.lookup 1 --package bundled:dictionary) 1
    assert equal (unanswered $ported dictionary.lookup 1 --package dictionary) 1
    assert equal (unanswered $ported dictionary.lookup 1) 1
    assert equal (unanswered $stand_in dictionary.lookup 1) 0

    # A variant that turns a package on beside a feature it doesn't answer must show the
    # package at the end of each pass: rows from it, or rows carrying its kind actions.
    let composed = [
        {
            searched: [apps.search]
            shown: {apps.search: 3}
        }
        {
            searched: [apps.search]
            shown: {apps.search: 3}
            composed: ["bundled:running-apps"]
        }
    ]
    assert equal (unenabled $composed ["bundled:running-apps"] 2) 0
    assert equal (unenabled $composed ["bundled:running-apps"] 1) 1 "a pass that shows it off counts"
    assert equal (unenabled ($composed | each { reject composed? }) ["bundled:running-apps"] 2) 1
    assert equal (unenabled $composed [] 1) 0
    let joined = [
        {
            searched: [apps.search, extension-search.repo]
            packages: {extension-search.repo: "bundled:repo"}
        }
    ]
    assert equal (unenabled $joined ["bundled:repo"] 1) 0 "a package joining the main search shows by its rows"
    assert equal (unenabled $joined ["bundled:repo", "bundled:running-apps"] 1) 1
    assert equal (enabled-grants ["bundled:running-apps"] | from toml) {
        version: 1
        packages: {"bundled:running-apps": {enabled: true}}
    }

    let ms = 1..100 | each { into float }
    assert equal ($ms | percentiles) {p50: 50.0, p95: 95.0, p99: 99.0}
    assert equal ([7.0] | percentiles) {p50: 7.0, p95: 7.0, p99: 7.0}

    let script = typing-script {prefix: "cal ", typed: "tom"} 2 0.05
    assert equal ($script.keys | length) 6
    let typed = $script.events | where type == type | get text | str join
    assert equal $typed "cal tomcal tom"
    # A pass resets and types the prefix at once, then each measured key a gap apart; the next
    # pass starts a gap after the last one's final key.
    let keys = $script.keys | each { math round --precision 3 }
    assert equal $keys [
        0.35
        0.4
        0.45
        0.55
        0.6
        0.65
    ]
    assert equal ($script.events | last | get name) end

    # A port names what it changes; the rest comes from the implementation it extends.
    let implementations = {
        native: {
            config: "native.toml"
            provider: snippets.search
            settings: {
                snippets: {sort: "name"}
            }
        }
        extension: {
            extends: native
            provider: extension.snippets
            extensions: "ports"
            settings: {
                extensions: true
                snippets: {enabled: false}
            }
        }
    }
    assert equal (resolve $implementations native) $implementations.native
    assert equal (resolve $implementations extension) {
        config: "native.toml"
        provider: extension.snippets
        settings: {snippets: {sort: "name", enabled: false}, extensions: true}
        extensions: "ports"
    }

    # Choosing one implementation keeps what it extends, so a variant of native measures alone
    # with native's config and provider, and each group of a display carries resolved setups.
    let features = {
        search: {
            native: {config: "native.toml", provider: apps.search}
            running-apps: {
                extends: native
                enables: ["bundled:running-apps"]
            }
            tall: {extends: native, display: "1728x2560"}
        }
    }
    assert equal (display-groups $features running-apps) [
        {
            display: ""
            features: {
                search: {
                    running-apps: {
                        config: "native.toml"
                        provider: apps.search
                        enables: ["bundled:running-apps"]
                        settings: {}
                    }
                }
            }
        }
    ]
    assert equal (display-groups $features tall | get 0.display) "1728x2560"
    assert equal (display-groups $features | each {|group| $group.features.search | columns }) [
        [native running-apps]
        [tall]
    ]
    assert equal (display-groups $features absent) []

    # Every implementation in the catalogue resolves, and every scenario names a feature it has.
    let catalogue = open ($env.FILE_PWD | path dirname | path join latency scenarios.toml)
    for feature in ($catalogue.features | transpose name implementations) {
        for label in ($feature.implementations | columns) {
            let setup = resolve $feature.implementations $label
            assert ($setup.config | is-not-empty) $"($feature.name).($label) has no config"
            assert ($setup.provider | is-not-empty) $"($feature.name).($label) has no provider"
            assert (
                ($label == "native") == ($setup.package? == null and $setup.enables? == null)
            ) $"($feature.name).($label) must name the package that answers it or the packages it turns on, and only a variant of native has one"
            assert ($setup.package? == null or $setup.enables? == null) $"($feature.name).($label) names both a package that answers it and packages it turns on"
        }
    }
    for scenario in $catalogue.scenarios {
        assert ($scenario.feature in ($catalogue.features | columns)) $"($scenario.id) has no feature"
    }

    # Every implementation besides native has something for `parity` to hold it to: the
    # feature's native implementation, a recorded native baseline of each of its scenarios, or
    # its own budget.
    let baseline = open ($env.FILE_PWD | path dirname | path join latency native-baseline.json)
    | get results
    | get id
    # Every recorded baseline says which benchmark measured it and what that run measured of the
    # control, or `parity` could never use it.
    assert equal (
        open ($env.FILE_PWD | path dirname | path join latency native-baseline.json)
        | get results
        | where {|entry| $entry.benchmark? == null or ($entry.control?.scenarios? | is-empty) }
        | get id
    ) []
    let control_ids = $catalogue.scenarios | where feature == $CONTROL | get id | sort
    assert equal (
        open ($env.FILE_PWD | path dirname | path join latency native-baseline.json)
        | get results
        | where {|entry| ($entry.control?.scenarios? | default [] | get id | sort) != $control_ids }
        | get id
    ) [] "every recorded baseline's control is the catalogue's control scenarios"
    for feature in ($catalogue.features | transpose name implementations) {
        let has_native = "native" in ($feature.implementations | columns)
        let scenarios = $catalogue.scenarios | where feature == $feature.name | get id
        let recorded = $scenarios | all {|id| $id in $baseline }
        for label in ($feature.implementations | columns | where $it != "native") {
            let setup = resolve $feature.implementations $label
            assert ($has_native or $recorded or $setup.budget_ms? != null) $"($feature.name).($label) has no native implementation, recorded native baseline or budget_ms"
        }
    }

    pull-request-report
}

def timed [
    id: string
    feature: string
    implementation: string
    p95: float
    merge: float
]: nothing -> record {
    {
        id: $id
        feature: $feature
        implementation: $implementation
        p95_ms: $p95
        stages: {
            dispatch_ms: {p50: 0.05, p95: 0.1}
            provider_ms: {p50: 0.2, p95: 0.4}
            merge_ms: {p50: 0.3, p95: $merge}
            layout_ms: {p50: 0.2, p95: 0.5}
        }
    }
}

def checked-timed [...rows: record]: nothing -> list<record> {
    $rows | checked-results
}

def json-block []: string -> record {
    $in | split row "```json\n" | get 1 | split row "\n```" | first | from json
}

# The pull request comment: main's results scaled by the native control as `parity` scales a
# recorded baseline, held to `compare`'s tolerance, and rendered for a person or an agent.
def pull-request-report [] {
    let control = {|p95: float|
        [
            (timed search.a $CONTROL native $p95 1.0)
            (timed search.b $CONTROL native $p95 1.0)
        ]
    }
    let main = (checked-timed
        ...(do $control 2.0)
        (timed x.k x extension 2.0 1.0)
        (timed gone.k gone extension 1.0 0.5)
    )
    let run = {|search: float, x: float, merge: float| (checked-timed
        ...(do $control $search)
        (timed x.k x extension $x $merge)
        (timed fresh.k fresh extension 1.0 0.5)
    ) }
    let tolerance = $COMPARE_TOLERANCE
    let floor = $COMPARE_FLOOR

    # A runner half as fast again measures everything half as slow again; scaled by the control,
    # that is no regression, though unscaled it would be.
    let slower_runner = against-main (do $run 3.0 3.2 1.5) $main $tolerance $floor
    assert equal $slower_runner.scale 1.5
    let x = $slower_runner.rows | where id == x.k | first
    assert equal ($x | select scale held_p95_ms limit_ms verdict) {scale: 1.5, held_p95_ms: 3.0, limit_ms: 4.0, verdict: ok}
    assert equal (
        against-main (do $run 3.0 3.2 1.5) ($main | where feature != $CONTROL) $tolerance $floor
        | get rows
        | where id == x.k
        | get 0.verdict
    ) regressed
    assert equal ($slower_runner.rows | get verdict) [ok ok ok new removed]
    assert equal ($slower_runner.rows | where id == fresh.k | get 0.pr_p95_ms) 1.0
    assert equal ($slower_runner.rows | where id == gone.k | get 0.main_p95_ms) 1.0

    # A regression on the same runner is flagged, with its stages scaled like the total.
    let slower_pr = against-main (do $run 3.0 4.3 2.9) $main $tolerance $floor
    let x = $slower_pr.rows | where id == x.k | first
    assert equal $x.verdict regressed
    assert equal ($x.delta_ms | math round --precision 3) 1.3
    assert equal ($x.delta_percent | math round --precision 2) 43.33
    assert equal (
        $x.stages
        | where stage == merge_ms
        | get 0
        | update cells { if ($in | describe) == float { math round --precision 3 } else { $in } }
    ) {
        stage: merge_ms
        main_ms: 1.5
        pr_ms: 2.9
        delta_ms: 1.4
    }

    # The control is what the scale is made of, so it is held to main's unscaled.
    let slower_control = against-main (do $run 3.5 2.0 1.0) $main $tolerance $floor
    let search = $slower_control.rows | where id == search.a | first
    assert equal ($search | select control scale limit_ms verdict) {control: true, scale: null, limit_ms: 3.0, verdict: regressed}

    # Nothing is scaled without the same control scenarios on both sides.
    assert equal (
        against-main (do $run 3.0 3.2 1.5) ($main | where id != search.b) $tolerance $floor
        | get scale
    ) null
    assert equal (
        against-main (checked-timed (timed x.k x extension 2.0 1.0)) $main $tolerance $floor
        | get scale
    ) null

    let run_record = {
        id: 7
        commit: ("b" | fill --character b --width 40)
        merge_base: true
        base: ("b" | fill --character b --width 40)
    }
    let context = {
        repo: "o/r"
        commit: ("c" | fill --character c --width 40)
        run_url: "https://example.com/run"
    }

    # Nothing regressed: one line, with the rest collapsed.
    let quiet = render $slower_runner $run_record $context
    let lines = $quiet | lines
    assert equal $lines.0 "<!-- dupin-latency -->"
    assert equal $lines.1 "### Keypress latency on ccccccc"
    assert equal $lines.3 "No scenario is slower than [main at bbbbbbb](https://github.com/o/r/actions/runs/7) beyond 10% or 1 ms, whichever is larger, after scaling by the native `search` control (×1.500)."
    assert equal $lines.5 "<details><summary>Every scenario</summary>"
    assert not ($quiet | str contains "What to do")
    assert (
        "| `x.k` (extension) | 2.00 ms | 3.00 ms | 3.20 ms | +0.20 ms (+6.7%) | 4.00 ms |  |" in $lines
    )
    assert (
        "| `search.a` (native, control) | 2.00 ms | 2.00 ms | 3.00 ms | +1.00 ms (+50.0%) | 3.00 ms |  |" in $lines
    )
    assert (
        "| `fresh.k` (extension) | – | – | 1.00 ms | – | – | new |" in $lines
    )
    assert (
        "| `gone.k` (extension) | 1.00 ms | – | – | – | – | removed |" in $lines
    )
    assert equal ($quiet | json-block | get regressed) []

    # A regression says what is slower, where the time went and what to do, and repeats it as JSON.
    let loud = render $slower_pr $run_record $context
    let lines = $loud | lines
    assert equal $lines.3 ":warning: **1 scenario is slower than main after scaling, beyond 10% or 1 ms, whichever is larger: `x.k`.** Latency never blocks merging; this comment is for whoever works on or reviews this pull request."
    assert (
        "- `x.k` (extension) p95 is 1.30 ms (43.3%) slower than main after scaling: 4.30 ms against a limit of 4.00 ms. The largest share is in `merge_ms` (+1.40 ms)." in $lines
    )
    assert ("| `merge_ms` | 1.50 ms | 2.90 ms | +1.40 ms |" in $lines)
    assert ($loud | str contains "`nu scripts/latency.nu run --only x --out")
    assert not ($loud | str contains "they are slower themselves")
    let summary = $loud | json-block
    assert equal ($summary | reject regressed) {
        schema: 1
        commit: $context.commit
        main: {commit: $run_record.commit, run_id: 7, merge_base: true}
        control: $CONTROL
        scale: 1.5
        tolerance_percent: 10.0
        floor_ms: 1.0
    }
    assert equal ($summary.regressed | select id scaled delta_ms largest_stage) [
        {
            id: x.k
            scaled: true
            delta_ms: 1.3
            largest_stage: merge_ms
        }
    ]
    assert equal $summary.regressed.0.stages.merge_ms {main_ms: 1.5, pr_ms: 2.9, delta_ms: 1.4}

    # A slower control warns that scaling may hide a slowdown every keypress pays for.
    assert (
        render $slower_control $run_record $context
        | str contains "they are slower themselves"
    )

    # Without main's results there is only this run's.
    let alone = render (against-main (do $run 3.0 3.2 1.5) null $tolerance $floor) null $context
    assert (
        $alone
        | str contains "This run carries no results from main to compare with"
    )
    assert not ($alone | str contains "Machine-readable")

    # The report reads a pull request's results with a token that can comment, so anything that
    # is not a scenario's name or a time is refused.
    let good = timed x.k x extension 2.0 1.0
    assert equal ([$good] | checked-results | get 0.stages) {
        dispatch_ms: {p95: 0.1}
        provider_ms: {p95: 0.4}
        merge_ms: {p95: 1.0}
        layout_ms: {p95: 0.5}
    }
    # `fetch-main` writes main's results checked, and `report` checks them again.
    assert equal ([$good] | checked-results | checked-results) ([$good] | checked-results)
    assert error { [($good | update id "[x](https://example.com)")] | checked-results }
    assert error { [($good | update implementation "native\n# heading")] | checked-results }
    assert error { [($good | update p95_ms "2.0")] | checked-results }
    assert error { [($good | update p95_ms (-1.0))] | checked-results }
    assert error { [($good | reject stages.merge_ms)] | checked-results }
    assert error { [$good $good] | checked-results }
    assert error { {results: []} | checked-results }
    let fetched = {
        schema: 1
        run: $run_record
        results: [$good]
    }
    assert equal ($fetched | checked-main | get run) $run_record
    assert error { $fetched | update run.commit "main" | checked-main }
    assert error { $fetched | update run.id "7" | checked-main }
    assert error { $fetched | update run.merge_base "yes" | checked-main }
    assert equal (
        $fetched
        | update run.base ""
        | checked-main
        | get run.base
    ) ""

    # `report` renders the files `run` and `fetch-main` write, and fails on one it refuses.
    let tmp = mktemp --directory
    let script = $env.FILE_PWD | path dirname | path join latency.nu
    let written = {|search: float, x: float, merge: float|
        [
            (timed search.a $CONTROL native $search 1.0)
            (timed search.b $CONTROL native $search 1.0)
            (timed x.k x extension $x $merge)
        ]
    }
    {
        schema: 1
        results: (do $written 3.0 4.3 2.9)
    } | to json | save ($tmp | path join results.json)
    {
        schema: 1
        run: $run_record
        results: (do $written 2.0 2.0 1.0)
    } | to json | save ($tmp | path join main.json)
    let report = ^$nu.current-exe $script report ($tmp | path join results.json) ($tmp | path join main.json) --repo o/r --commit $context.commit
    assert ($report | str contains "#### What to do")
    {
        schema: 1
        results: [
            ($good | update id "<img src=x>")
        ]
    } | to json | save ($tmp | path join forged.json)
    let forged = do { ^$nu.current-exe $script report ($tmp | path join forged.json) } | complete
    assert not equal $forged.exit_code 0
    rm --recursive $tmp
}
