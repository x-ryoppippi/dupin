use std/assert
use ../latency.nu [percentiles typing-script unanswered verdicts]

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

    # A keypress counts as answered only when the feature's provider shows rows for it.
    let samples = [
        {
            shown: {web.keywords: 1, apps.search: 3}
        }
        {
            shown: {apps.search: 4}
        }
        {
            shown: {}
        }
        {}
    ]
    assert equal (unanswered $samples web.keywords) 3
    assert equal (unanswered $samples apps.search) 2

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
}
