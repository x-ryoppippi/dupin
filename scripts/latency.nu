#!/usr/bin/env nu

# Keypress-to-results latency: how long Dupin takes from a typed key to the first frame
# showing every provider's results for the new query.
#
# `run` measures each scenario in scripts/latency/scenarios.toml headless, with fixtures,
# and writes the percentiles as JSON. `compare` holds one result set against another (main
# against a pull request), and `parity` holds each feature's other implementations against
# its native one within a result set. See docs/development.md, "Keypress latency".
def main []: nothing -> nothing {
    help main
}

# Measure every scenario with every implementation its feature declares.
#
# Each scenario runs `--runs` times, each in a fresh process. A run types the scenario
# `--warmup` times unmeasured, then enough times for `--samples` measured keypresses, each
# `--gap` after the last. The reported percentiles are the median of the runs' own.
def "main run" [
    --bin: path = "target/latency/dupin" # `cargo build --profile latency -p dupin --features tooling`, or a debug build
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
    let catalogue = open scripts/latency/scenarios.toml
    let jobs = $catalogue.scenarios
    | where {|scenario| $only == null or ($scenario.id | str contains $only) }
    | each {|scenario|
        let implementations = $catalogue.features | get $scenario.feature
        $implementations
        | columns
        | where {|label| $implementation == null or $label == $implementation }
        | each {|label|
            $scenario | insert implementation $label | insert setup (resolve $implementations $label)
        }
    }
    | flatten
    if ($jobs | is-empty) {
        error make {msg: "no scenario matches"}
    }
    let settings = {
        samples: $samples
        warmup: $warmup
        runs: $runs
        gap_ms: ($gap / 1ms)
        fps: $fps
    }
    let tmp = mktemp --directory
    # Round by round rather than scenario by scenario, so whatever else the machine is doing
    # weighs on every implementation alike.
    let measured = 1..$runs | each {|run|
        $jobs | enumerate | each {|job|
            let name = $"($job.item.id) [($job.item.implementation)]"
            print --stderr $"run ($run)/($runs): ($name)"
            let dir = $tmp | path join $"($job.item.id)-($job.item.implementation)-($run)"
            {job: $job.index, run: (measure $bin $job.item $settings $dir)}
        }
    } | flatten
    let results = $jobs | enumerate | each {|job|
        summarize $job.item ($measured | where job == $job.index | get run)
    }
    rm --recursive $tmp
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

# Hold every implementation other than `native` against the native one of the same feature,
# within one result set. Passes when no feature declares a second implementation.
def "main parity" [
    results: path
    --tolerance: float = 10.0 # Percent of native's p95
    --floor: duration = 1ms # The least regression ever allowed
    --summary: path # Also append the comparison to this Markdown file
]: nothing -> nothing {
    let all = open $results | get results
    let pairs = $all | where implementation != "native" | each {|result|
        {
            candidate: $result
            baseline: ($all | where id == $result.id and implementation == "native" | get 0?)
        }
    }
    if ($pairs | is-empty) {
        print --stderr "no feature declares an implementation besides native; nothing to hold to parity"
        return
    }
    judge $pairs $tolerance $floor false $summary "Extension parity"
}

# The implementation `label` of a feature, with what it `extends` filled in: a port names only
# what it changes, such as its extensions and the `settings` that hand the feature to them,
# merged over the config it starts from.
export def resolve [implementations: record, label: string]: nothing -> record {
    let own = $implementations | get $label
    if $own.extends? == null {
        return $own
    }
    let base = resolve $implementations $own.extends
    let settings = $base.settings? | default {} | merge deep ($own.settings? | default {})
    $base | merge ($own | reject extends) | upsert settings $settings
}

# One process: type the scenario, then read back the samples for the measured keypresses.
def measure [
    bin: path
    job: record
    settings: record
    dir: path
]: nothing -> record {
    mkdir $dir
    let gap = $settings.gap_ms / 1000
    let per_pass = $job.typed | split chars | length
    let passes = $settings.warmup + ($settings.samples / $per_pass | math ceil)
    let script = typing-script $job $passes $gap
    let script_path = $dir | path join script.json
    {events: $script.events} | to json | save $script_path
    let config = $dir | path join config
    mkdir ($config | path join dupin)
    let target = $config | path join dupin config.toml
    if $job.setup.settings? == null {
        cp $job.setup.config $target
    } else {
        open $job.setup.config | merge deep $job.setup.settings | to toml | save $target
    }
    if $job.setup.extensions? != null {
        cp --recursive $job.setup.extensions ($config | path join dupin extensions)
    }
    let log = $dir | path join samples.jsonl
    let env_vars = {
        HOME: $dir
        XDG_CONFIG_HOME: $config
        XDG_DATA_HOME: ($dir | path join data)
        DUPIN_LATENCY: $log
        DUPIN_HEADLESS_CLIPBOARD: ($env.PWD | path join scripts/fixtures/clipboard.json)
    }
    let result = with-env $env_vars {
        ^$bin --script $script_path --realtime --fps $settings.fps --tree ($dir | path join tree.json) --at mark:end
        | complete
    }
    if $result.exit_code != 0 {
        error make {msg: $"($bin) failed on ($job.id):\n($result.stderr)"}
    }
    let lines = open --raw $log | lines | each { from json }
    let measured = $script.keys | skip ($settings.warmup * $per_pass) | first $settings.samples
    let matched = $measured | each {|t|
        $lines | where {|line| $line.t >= $t - 1e-6 and $line.t < $t + $gap / 2 } | get 0?
    }
    let unseen = $matched | where $it == null | length
    if $unseen > 0 {
        error make {
            msg: $"($job.id): ($unseen) measured keypresses left no sample; is ($bin) built with the latency probe?"
        }
    }
    let superseded = $matched | where superseded? == true
    # A keypress whose results were never drawn has no latency to report; more than a few
    # mean the gap is shorter than the scenario takes.
    if ($superseded | length) * 100 > ($matched | length) {
        error make {
            msg: $"($job.id): ($superseded | length) of ($matched | length) keypresses were typed over before their results showed; raise --gap"
        }
    }
    let provider = $job.setup.provider
    let missed = unanswered $matched $provider $per_pass
    if $missed > 0 {
        error make {
            msg: $"($job.id) [($job.implementation)]: ($missed) measured keypresses did not search ($provider), or ended a pass without its rows; the scenario does not reach its feature"
        }
    }
    let samples = $matched | where superseded? != true
    {
        commands: ($samples | get command | uniq)
        samples: ($samples | length)
        superseded: ($superseded | length)
        total_ms: ($samples | get total_ms | percentiles)
        stages: (
            stage-names
            | each {|stage| {$stage: ($samples | get $stage | percentiles)} }
            | into record
        )
    }
}

# How many drawn keypresses, `per_pass` to a pass, did not search `provider`, or ended a pass
# without a row from it. Only the whole of `typed` must show the feature's rows: a shorter
# query may rightly have none.
export def unanswered [samples: list<record>, provider: string, per_pass: int]: nothing -> int {
    $samples
    | enumerate
    | where {|it| $it.item.superseded? != true }
    | where {|it|
        let searched = $provider in ($it.item.searched? | default [])
        let shown = $it.item.shown?
        | default {}
        | transpose name rows
        | where name == $provider and rows > 0
        | is-not-empty
        let ends_pass = ($it.index + 1) mod $per_pass == 0
        not $searched or ($ends_pass and not $shown)
    }
    | length
}

def stage-names []: nothing -> list<string> {
    [dispatch_ms provider_ms merge_ms layout_ms]
}

# The scripted keys: per pass, select all and delete twice (back to the default search) and
# type the unmeasured prefix, all in one frame, then each measured character `gap` apart.
export def typing-script [job: record, count: int, gap: float]: nothing -> record {
    let typed = $job.typed | split chars
    let prefix = $job.prefix | split chars
    let pass_length = ($typed | length) + 1
    let start = 0.3
    let passes = 0..<$count | each {|pass|
        let t0 = $start + $pass * $pass_length * $gap
        let reset = [
            {t: $t0, type: key, name: a, cmd: true}
            {t: $t0, type: key, name: Backspace}
            {t: $t0, type: key, name: Backspace}
        ]
        let prefix_keys = $prefix | each {|ch| {t: $t0, type: type, text: $ch} }
        let keys = $typed | enumerate | each {|it| {t: ($t0 + (1 + $it.index) * $gap), type: type, text: $it.item} }
        {events: ($reset ++ $prefix_keys ++ $keys), keys: ($keys | get t)}
    }
    let end = $start + $count * $pass_length * $gap + $gap
    {
        events: (
            $passes
            | get events
            | flatten
            | append {t: $end, type: mark, name: end}
        )
        keys: ($passes | get keys | flatten)
    }
}

# Nearest-rank p50, p95 and p99 of a list of milliseconds.
export def percentiles []: list<number> -> record {
    let sorted = $in | sort
    let count = $sorted | length
    let at = {|p|
        $sorted | get ([
            ((($count * $p) | math ceil) - 1)
            0
        ] | math max)
    }
    {
        p50: (do $at 0.5)
        p95: (do $at 0.95)
        p99: (do $at 0.99)
    }
}

# One scenario's runs as one result: each percentile is the median of the runs' own, which
# keeps one disturbed run on a shared CI machine from moving the result.
def summarize [job: record, runs: list<record>]: nothing -> record {
    let median = {|get_value|
        $runs | each $get_value | math median
    }
    {
        id: $job.id
        feature: $job.feature
        implementation: $job.implementation
        prefix: $job.prefix
        typed: $job.typed
        provider: $job.setup.provider
        commands: (
            $runs
            | get commands
            | flatten
            | uniq
        )
        samples: ($runs | get samples | math sum)
        superseded: ($runs | get superseded | math sum)
        p50_ms: (do $median {|run| $run.total_ms.p50 })
        p95_ms: (do $median {|run| $run.total_ms.p95 })
        p99_ms: (do $median {|run| $run.total_ms.p99 })
        stages: (stage-names | each {|stage|
            {
                $stage: {
                    p50: (do $median {|run| $run.stages | get $stage | get p50 })
                    p95: (do $median {|run| $run.stages | get $stage | get p95 })
                }
            }
        } | into record)
        runs: ($runs | each {|run| $run.total_ms })
    }
}

# Each pair's verdict: `regressed` when the candidate's p95 exceeds the baseline's by more than
# `tolerance` percent of it or `floor`, whichever is larger; `new` without a baseline.
export def verdicts [pairs: list<record>, tolerance: float, floor: duration]: nothing -> table {
    $pairs | each {|pair|
        let cand = $pair.candidate
        match $pair.baseline {
            null => {
                id: $cand.id
                baseline: null
                candidate: $cand.implementation
                baseline_p95_ms: null
                candidate_p95_ms: ($cand.p95_ms | math round --precision 2)
                allowed_ms: null
                verdict: "new"
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
