#!/usr/bin/env nu

# Keypress-to-results latency: how long Dupin takes from a typed key to the first frame
# showing every provider's results for the new query.
#
# `run` measures each scenario in scripts/latency/scenarios.toml headless, with fixtures,
# and writes the percentiles as JSON. `compare` holds one result set against another (main
# against a pull request), and `parity` holds each feature's other implementations against
# its native one: the same run's, or the one `baseline` recorded from the last commit that had
# it. See docs/development.md, "Keypress latency".
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

# The least regression `parity` allows. Native p95 varies by about a quarter of a millisecond
# between CI runs, so `compare`'s 1 ms would let a port of a scenario that takes 1.4 ms be 70%
# slower and pass.
export const PARITY_FLOOR = 500us

# Hold every implementation other than `native` against the native one of the same scenario:
# the run's own, or, for a feature whose native implementation is gone, the one recorded in
# `--baseline` from the last commit that had it. One that never had a native implementation is
# held to its own `budget_ms`. Passes when nothing has a native or a budget to match.
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
        {candidate: $result, baseline: ($native | default $earlier)}
    }
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
        {
            id: $id
            feature: $feature
            implementation: "native"
            commit: $commits.0
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
    let clipboard = match $job.setup.clipboard_entries? {
        null => ($env.PWD | path join scripts/fixtures/clipboard.json)
        $count => {
            let path = $dir | path join clipboard.json
            clipboard-fixture $count | to json | save --force $path
            $path
        }
    }
    let env_vars = {
        HOME: $dir
        XDG_CONFIG_HOME: $config
        XDG_DATA_HOME: ($dir | path join data)
        DUPIN_LATENCY: $log
        DUPIN_HEADLESS_CLIPBOARD: $clipboard
    }
    let bin = if $job.setup.bundled? == true { app-with-bundled $bin $dir } else { $bin }
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
    let package = $job.setup.package?
    let missed = unanswered $matched $provider $per_pass --package $package
    if $missed > 0 {
        let rows = if $package == null { "its rows" } else { $"its rows from the package ($package)" }
        error make {
            msg: $"($job.id) [($job.implementation)]: ($missed) measured keypresses did not search ($provider), or ended a pass without ($rows); the scenario does not reach its feature"
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

# A copy of `bin` inside a Dupin.app in `dir` whose Resources hold the repository's
# `extensions/`, the one place a binary built outside the source tree finds bundled extensions,
# as the released app finds its own.
def app-with-bundled [bin: path, dir: path]: nothing -> string {
    let contents = $dir | path join Dupin.app Contents
    mkdir ($contents | path join MacOS) ($contents | path join Resources)
    let app_bin = $contents | path join MacOS dupin
    cp $bin $app_bin
    cp --recursive extensions ($contents | path join Resources extensions)
    $app_bin
}

# `count` clipboard history entries, oldest first, as DUPIN_HEADLESS_CLIPBOARD lists them: text of
# a few lengths and some copied files, a few mentioning Baker Street, each copied half a minute
# after the one before.
export def clipboard-fixture [count: int]: nothing -> list<record> {
    let words = [
        alpha
        bravo
        charlie
        delta
        echo
        foxtrot
        golf
        hotel
        india
        juliet
        kilo
        lima
        mike
        november
        oscar
        papa
        quebec
        romeo
        sierra
        tango
        uniform
        victor
        whiskey
        xray
        yankee
        zulu
    ]
    0..<$count | each {|index|
        let age = ($count - $index) * 30
        if $index mod 50 == 7 {
            {files: [$"/Users/Shared/Fixtures/Report ($index).pdf"], age: $age}
        } else {
            let picked = 0..($index mod 12 + 2) | each {|offset| $words | get (($index * 7 + $offset * 3) mod ($words | length)) }
            let street = if $index mod 37 == 0 { " Meet at Baker Street 221B." } else { "" }
            {text: $"Note ($index): ($picked | str join ' ')($street)", age: $age}
        }
    }
}

# How many drawn keypresses, `per_pass` to a pass, did not search `provider`, or ended a pass
# without a row from it. Only the whole of `typed` must show the feature's rows: a shorter
# query may rightly have none. With `package`, those rows must include the package's own, so a
# feature an extension answers can't pass on the rows its command shows when the package is
# broken or missing.
export def unanswered [
    samples: list<record>
    provider: string
    per_pass: int
    --package: string
]: nothing -> int {
    let has_rows = {|counts: any, name: string|
        $counts | default {} | transpose name rows | where name == $name and rows > 0 | is-not-empty
    }
    $samples
    | enumerate
    | where {|it| $it.item.superseded? != true }
    | where {|it|
        let searched = $provider in ($it.item.searched? | default [])
        let shown = (do $has_rows $it.item.shown? $provider) and (
            $package == null or (do $has_rows $it.item.packages? $package)
        )
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
        budget_ms: $job.setup.budget_ms?
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
