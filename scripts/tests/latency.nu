use std/assert
use ../latency.nu [percentiles resolve typing-script unanswered verdicts]

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
    # from the stand-in that answers while the package isn't on.
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
            assert (($label == "native") == ($setup.package? == null)) $"($feature.name).($label) must name the package that answers it, and only an extension has one"
        }
    }
    for scenario in $catalogue.scenarios {
        assert ($scenario.feature in ($catalogue.features | columns)) $"($scenario.id) has no feature"
    }
}
