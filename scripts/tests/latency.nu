use std/assert
use ../latency.nu [
    PARITY_FLOOR
    parity-pairs
    stale
    unheld
    unmeasured
    verdicts
]
use ../latency/measure.nu [
    fingerprint
    percentiles
    resolve
    typing-script
    unanswered
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

    # Every implementation in the catalogue resolves, and every scenario names a feature it has.
    let catalogue = open ($env.FILE_PWD | path dirname | path join latency scenarios.toml)
    for feature in ($catalogue.features | transpose name implementations) {
        for label in ($feature.implementations | columns) {
            let setup = resolve $feature.implementations $label
            assert ($setup.config | is-not-empty) $"($feature.name).($label) has no config"
            assert ($setup.provider | is-not-empty) $"($feature.name).($label) has no provider"
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
    # Every recorded baseline says which benchmark measured it, or `parity` could never use it.
    assert equal (
        open ($env.FILE_PWD | path dirname | path join latency native-baseline.json)
        | get results
        | where benchmark? == null
        | get id
    ) []
    for feature in ($catalogue.features | transpose name implementations) {
        let has_native = "native" in ($feature.implementations | columns)
        let scenarios = $catalogue.scenarios | where feature == $feature.name | get id
        let recorded = $scenarios | all {|id| $id in $baseline }
        for label in ($feature.implementations | columns | where $it != "native") {
            let setup = resolve $feature.implementations $label
            assert ($has_native or $recorded or $setup.budget_ms? != null) $"($feature.name).($label) has no native implementation, recorded native baseline or budget_ms"
        }
    }
}
