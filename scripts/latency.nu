#!/usr/bin/env nu

# Keypress-to-results latency: how long Dupin takes from a typed key to the first frame
# showing every provider's results for the new query.
#
# `run` measures each scenario in scripts/latency/scenarios.toml headless, with fixtures,
# and writes the percentiles as JSON. `compare` holds one result set against another (main
# against a pull request), and `parity` holds each feature's other implementations against
# its native one: the same run's, or the one `baseline` recorded from the last commit that had
# it. See docs/development.md, "Keypress latency".

use latency/measure.nu [measure-all stage-names]

def main []: nothing -> nothing {
    help main
}

# Measure every scenario with every implementation its feature declares.
#
# Each scenario runs `--runs` times, each in a fresh process. A run types the scenario
# `--warmup` times unmeasured, then enough times for `--samples` measured keypresses, each
# `--gap` after the last. The reported percentiles are the median of the runs' own.
def "main run" [
    --bin: path = "target/latency/dupin" # `cargo build --profile latency -p dupin`, or a debug build
    --out: path = "target/latency/results.json"
    --samples: int = 210 # Measured keypresses per scenario and run
    --warmup: int = 10 # Unmeasured repetitions of each scenario before the measured ones
    --runs: int = 3
    --gap: duration = 25ms # Between keypresses; longer than the slowest one takes
    --fps: int = 1000 # Headless frame rate; a frame starts at most this often
    --only: string # Only the scenarios whose id contains this
    --implementation: string # Only this implementation of each feature
]: nothing -> nothing {
    let root = $env.FILE_PWD | path dirname
    cd $root
    let settings = {
        samples: $samples
        warmup: $warmup
        runs: $runs
        gap_ms: ($gap / 1ms)
        fps: $fps
    }
    let results = (
        measure-all (open scripts/latency/scenarios.toml) $bin $settings
        --only $only --implementation $implementation
    )
    let report = {
        schema: 1
        created: (date now | format date "%+")
        commit: (
            ^git rev-parse HEAD
            | complete
            | get stdout
            | str trim
        )
        settings: $settings
        results: $results
    }
    mkdir ($out | path dirname)
    $report | to json | save --force $out
    print --stderr $"wrote ($out)"
    print (
        $results | select id implementation samples p50_ms p95_ms p99_ms
        | update cells --columns [p50_ms p95_ms p99_ms] { math round --precision 2 }
        | table
    )
}

# Hold a candidate result set against a baseline, scenario by scenario.
#
# A scenario regresses when the candidate's p95 exceeds the baseline's by more than
# `--tolerance` percent of it or `--floor`, whichever is larger. Without the implementation
# flags each scenario is matched against the same implementation in the baseline.
def "main compare" [
    baseline: path
    candidate: path
    --tolerance: float = 10.0 # Percent of the baseline's p95
    --floor: duration = 1ms # The least regression ever allowed
    --baseline-implementation: string
    --candidate-implementation: string
    --report # Print the comparison without failing on a regression
    --summary: path # Also append the comparison to this Markdown file
]: nothing -> nothing {
    let base = open $baseline | get results
    let cand = open $candidate | get results
    let pairs = $cand
    | where {|result| $candidate_implementation == null or $result.implementation == $candidate_implementation }
    | each {|result|
        let label = $baseline_implementation | default $result.implementation
        {
            candidate: $result
            baseline: ($base | where id == $result.id and implementation == $label | get 0?)
        }
    }
    judge $pairs $tolerance $floor $report $summary "Keypress latency"
}

# The least regression `parity` allows. Native p95 varies by about a quarter of a millisecond
# between CI runs, so `compare`'s 1 ms would let a port of a scenario that takes 1.4 ms be 70%
# slower and pass.
export const PARITY_FLOOR = 500us

# Hold every implementation other than `native` against the native one of the same scenario:
# the run's own, or, for a feature whose native implementation is gone, the one recorded in
# `--baseline` from the last commit that had it. One that never had a native implementation is
# held to its own `budget_ms`. One with none of these fails, so a port can't skip the gate by
# deleting its native implementation without recording a baseline. A recorded one counts only
# when the benchmark that measured it is the one this run measured the port with.
def "main parity" [
    results: path
    --baseline: path # Native results recorded from earlier commits; scripts/latency/native-baseline.json
    --tolerance: float = 10.0 # Percent of native's p95
    --floor: duration = $PARITY_FLOOR # The least regression ever allowed
    --summary: path # Also append the comparison to this Markdown file
]: nothing -> nothing {
    let all = open $results | get results
    let recorded = open ($baseline | default ($env.FILE_PWD | path join latency native-baseline.json))
    | get results
    let unmeasured = unmeasured $all $recorded
    if ($unmeasured | is-not-empty) {
        error make {
            msg: $"($unmeasured | str join ', ') has a recorded native baseline but no result in ($results); a port must be measured against it"
        }
    }
    let pairs = parity-pairs $all $recorded
    let unheld = unheld $pairs
    if ($unheld | is-not-empty) {
        error make {
            msg: $"($unheld | str join ', ') has no native result, recorded native baseline or budget_ms to be held to; record the baseline of the native implementation it replaces, or declare a budget for a feature that never had one"
        }
    }
    let stale = stale $pairs
    if ($stale | is-not-empty) {
        error make {
            msg: $"($stale | str join ', ') has a native baseline recorded by another version of the benchmark than this run's; re-record it with this version, as docs/development.md, \"Native baselines\", describes"
        }
    }
    if ($pairs | is-empty) {
        print --stderr "no feature declares an implementation besides native; nothing to hold to parity"
        return
    }
    judge $pairs $tolerance $floor false $summary "Extension parity"
}

# Each implementation other than `native` in `results`, paired with the run's native result of
# its scenario or, failing that, the one `recorded` from the last commit that had it.
export def parity-pairs [results: list<record>, recorded: list<record>]: nothing -> list<record> {
    $results | where implementation != "native" | each {|result|
        let native = $results | where id == $result.id and implementation == "native" | get 0?
        let earlier = $recorded
        | where id == $result.id
        | get 0?
        | if $in != null { upsert implementation $"native at ($in.commit | str substring 0..<7)" } else { $in }
        {
            candidate: $result
            baseline: ($native | default $earlier)
            recorded: ($native == null and $earlier != null)
        }
    }
}

# The scenarios of `pairs` held to a recorded native result that another version of the
# benchmark measured: its `fingerprint` differs from the candidate's, or it predates them.
export def stale [pairs: list<record>]: nothing -> list<string> {
    $pairs
    | where {|pair|
        $pair.recorded and ($pair.baseline.benchmark? == null or $pair.baseline.benchmark != $pair.candidate.benchmark?)
    }
    | each {|pair| $"($pair.candidate.id) [($pair.candidate.implementation)]" }
}

# The scenarios of `pairs` with nothing to hold them to: no native result, no recorded one and no
# `budget_ms`. `compare` reports such a scenario as `new`, but for `parity` it is a port whose
# native implementation went without its baseline being recorded.
export def unheld [pairs: list<record>]: nothing -> list<string> {
    $pairs
    | where {|pair| $pair.baseline == null and $pair.candidate.budget_ms? == null }
    | each {|pair| $"($pair.candidate.id) [($pair.candidate.implementation)]" }
}

# The recorded scenarios `results` has no result for. In CI that is a port whose scenario was
# dropped or renamed, which would otherwise go unmeasured.
export def unmeasured [results: list<record>, recorded: list<record>]: nothing -> list<string> {
    let ids = $results | get id
    $recorded | get id | where {|id| $id not-in $ids }
}

# Record the native results of `feature` in the native baseline that `parity` holds its port
# to once the native implementation is deleted. `results` are `run` outputs of the last commit
# that has it, measured by the Latency workflow (see docs/development.md, "Native baselines");
# each figure is the median of theirs.
def "main baseline" [
    feature: string
    ...results: path
    --out: path # scripts/latency/native-baseline.json
]: nothing -> nothing {
    let out = $out | default ($env.FILE_PWD | path join latency native-baseline.json)
    if ($results | is-empty) {
        error make {msg: "name at least one results.json"}
    }
    let reports = $results | each { open }
    let commits = $reports | get commit | uniq
    if ($commits | length) != 1 {
        error make {
            msg: $"the results come from more than one commit: ($commits | str join ', ')"
        }
    }
    let measured = $reports
    | each {|report| $report.results | where feature == $feature and implementation == "native" }
    | flatten
    if ($measured | is-empty) {
        error make {msg: $"the results have no native ($feature)"}
    }
    let median = {|rows: list<record>, get_value: closure|
        $rows | each $get_value | math median
    }
    let recorded = $measured | group-by id | items {|id, rows|
        let benchmarks = $rows | get --optional benchmark | uniq
        if ($benchmarks | length) != 1 or $benchmarks.0 == null {
            error make {
                msg: $"($id) was not measured by one fingerprinted version of the benchmark; measure a commit whose scripts/latency/measure.nu records `benchmark`"
            }
        }
        {
            id: $id
            feature: $feature
            implementation: "native"
            commit: $commits.0
            benchmark: $benchmarks.0
            runs: ($rows | length)
            p50_ms: (do $median $rows {|row| $row.p50_ms })
            p95_ms: (do $median $rows {|row| $row.p95_ms })
            p99_ms: (do $median $rows {|row| $row.p99_ms })
            stages: (stage-names | each {|stage|
                {
                    $stage: {
                        p50: (do $median $rows {|row| $row.stages | get $stage | get p50 })
                        p95: (do $median $rows {|row| $row.stages | get $stage | get p95 })
                    }
                }
            } | into record)
        }
    }
    let kept = if ($out | path exists) {
        open $out | get results | where feature != $feature
    } else {
        []
    }
    {
        schema: 1
        results: ($kept ++ $recorded | sort-by id)
    }
    | to json --tabs 1
    | $in + "\n"
    | save --force $out
    print --stderr $"recorded ($recorded | length) scenarios of ($feature) from ($commits.0) in ($out)"
}

# Each pair's verdict: `regressed` when the candidate's p95 exceeds the baseline's by more than
# `tolerance` percent of it or `floor`, whichever is larger. Without a baseline, `regressed` when
# it exceeds the candidate's own `budget_ms`, and `new` when it has none.
export def verdicts [pairs: list<record>, tolerance: float, floor: duration]: nothing -> table {
    $pairs | each {|pair|
        let cand = $pair.candidate
        match $pair.baseline {
            null => {
                let budget = $cand.budget_ms?
                {
                    id: $cand.id
                    baseline: (if $budget == null { null } else { "budget" })
                    candidate: $cand.implementation
                    baseline_p95_ms: null
                    candidate_p95_ms: ($cand.p95_ms | math round --precision 2)
                    allowed_ms: $budget
                    verdict: (
                        if $budget == null { "new" } else if $cand.p95_ms > $budget { "regressed" } else { "ok" }
                    )
                }
            }
            $base => {
                let allowed = [($base.p95_ms * $tolerance / 100) ($floor / 1ms)] | math max
                {
                    id: $cand.id
                    baseline: $base.implementation
                    candidate: $cand.implementation
                    baseline_p95_ms: ($base.p95_ms | math round --precision 2)
                    candidate_p95_ms: ($cand.p95_ms | math round --precision 2)
                    allowed_ms: ($allowed | math round --precision 2)
                    verdict: (if $cand.p95_ms > $base.p95_ms + $allowed { "regressed" } else { "ok" })
                }
            }
        }
    }
}

def judge [
    pairs: list<record>
    tolerance: float
    floor: duration
    report: bool
    summary
    title: string
]: nothing -> nothing {
    let rows = verdicts $pairs $tolerance $floor
    print ($rows | table)
    if $summary != null {
        $"## ($title)\n\np95 keypress-to-results latency; a regression is more than ($tolerance)% or ($floor), whichever is larger.\n\n($rows | to md)\n\n"
        | save --append $summary
    }
    let regressed = $rows | where verdict == "regressed"
    if not $report and ($regressed | is-not-empty) {
        error make {
            msg: $"p95 latency regressed beyond tolerance: ($regressed | get id | str join ', ')"
        }
    }
}
