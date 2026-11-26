#!/usr/bin/env nu

# Keypress-to-results latency: how long Dupin takes from a typed key to the first frame
# showing every provider's results for the new query.
#
# `run` measures each scenario in scripts/latency/scenarios.toml headless, with fixtures,
# and writes the percentiles as JSON. `compare` holds one result set against another (main
# against a pull request), and `parity` holds each feature's other implementations against
# its native one: the same run's, or the one `baseline` recorded from the last commit that had
# it, scaled by how fast this run's machine measured the native control against that one's.
# `fetch-main` downloads the results of the main build a pull request merged onto, and `report`
# renders the pull request's against them, scaled by the control the same way, as the Markdown
# of the Latency comment workflow's comment; it never fails on a regression. See
# docs/development.md, "Keypress latency".

use latency/measure.nu [measure-all stage-names]
use main-build.nu find-main-artifact

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

# The regression `compare` and `report` allow: this percent of the baseline's p95 or the floor,
# whichever is larger.
export const COMPARE_TOLERANCE = 10.0
export const COMPARE_FLOOR = 1ms

# Hold a candidate result set against a baseline, scenario by scenario.
#
# A scenario regresses when the candidate's p95 exceeds the baseline's by more than
# `--tolerance` percent of it or `--floor`, whichever is larger. Without the implementation
# flags each scenario is matched against the same implementation in the baseline.
def "main compare" [
    baseline: path
    candidate: path
    --tolerance: float = $COMPARE_TOLERANCE # Percent of the baseline's p95
    --floor: duration = $COMPARE_FLOOR # The least regression ever allowed
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

# The native feature every run measures, so a baseline recorded on one machine can be held to a
# run on another: the recorded p95 is scaled by this run's control p95 over the one recorded
# with it. Runners of one class differ in speed by more than parity's tolerance.
export const CONTROL = "search"

# Hold every implementation other than `native` against the native one of the same scenario:
# the run's own, or, for a feature whose native implementation is gone, the one recorded in
# `--baseline` from the last commit that had it. One that never had a native implementation is
# held to its own `budget_ms`. One with none of these fails, so a port can't skip the gate by
# deleting its native implementation without recording a baseline. A recorded one counts only
# when the benchmark that measured it is the one this run measured the port with, and only with
# the `CONTROL` recorded beside it and measured by this run, which scale it to this machine.
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
    let uncontrolled = uncontrolled $pairs
    if ($uncontrolled | is-not-empty) {
        error make {
            msg: $"($uncontrolled | str join ', ') has a native baseline that cannot be scaled to this run: it was recorded without the native ($CONTROL) scenarios, or ($results) lacks one of them; re-record it, or measure them"
        }
    }
    if ($pairs | is-empty) {
        print --stderr "no feature declares an implementation besides native; nothing to hold to parity"
        return
    }
    judge $pairs $tolerance $floor false $summary "Extension parity"
}

# Each implementation other than `native` in `results`, paired with the run's native result of
# its scenario or, failing that, the one `recorded` from the last commit that had it. A recorded
# one comes with its `control`: the control scenarios recorded beside it, each with this run's
# result of it, and `scale`, this run's control p95 over the recorded one, once the run has all.
export def parity-pairs [results: list<record>, recorded: list<record>]: nothing -> list<record> {
    $results | where implementation != "native" | each {|result|
        let native = $results | where id == $result.id and implementation == "native" | get 0?
        let earlier = $recorded
        | where id == $result.id
        | get 0?
        | if $in != null { upsert implementation $"native at ($in.commit | str substring 0..<7)" } else { $in }
        let held = $native == null and $earlier != null
        {
            candidate: $result
            baseline: ($native | default $earlier)
            recorded: $held
            control: (if $held { control-of $results $earlier.control? } else { null })
        }
    }
}

def control-of [results: list<record>, recorded]: nothing -> any {
    if $recorded == null {
        return null
    }
    let scenarios = $recorded.scenarios | each {|scenario|
        $scenario
        | insert measured ($results | where id == $scenario.id and implementation == "native" | get 0?)
    }
    let complete = ($scenarios | is-not-empty) and ($scenarios | all {|scenario| $scenario.measured != null })
    {
        scenarios: $scenarios
        scale: (
            if $complete { ($scenarios | get measured.p95_ms | math median) / $recorded.p95_ms } else { null }
        )
    }
}

# The scenarios of `pairs` held to a recorded native result that another version of the
# benchmark measured: its `fingerprint` differs from the candidate's, or it predates them, or one
# of its control scenarios' differs from this run's.
export def stale [pairs: list<record>]: nothing -> list<string> {
    $pairs
    | where {|pair|
        let control = $pair.control?.scenarios? | default []
        $pair.recorded and (
            $pair.baseline.benchmark? == null
            or $pair.baseline.benchmark != $pair.candidate.benchmark?
            or ($control | any {|scenario| $scenario.measured != null and $scenario.measured.benchmark? != $scenario.benchmark })
        )
    }
    | each {|pair| $"($pair.candidate.id) [($pair.candidate.implementation)]" }
}

# The scenarios of `pairs` held to a recorded native result that cannot be scaled to this run:
# it was recorded without a control, or the run lacks one of its control scenarios.
export def uncontrolled [pairs: list<record>]: nothing -> list<string> {
    $pairs
    | where {|pair| $pair.recorded and $pair.control?.scale? == null }
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
# that has it (see docs/development.md, "Native baselines"); each figure is the median of theirs.
# Beside them it records what the same results measured of the native `CONTROL` scenarios, which
# `parity` scales the baseline by.
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
    let without = $results | zip $reports | where {|pair|
        $pair.1.results | where feature == $feature and implementation == "native" | is-empty
    }
    if ($without | is-not-empty) {
        error make {
            msg: $"($without | each { get 0 } | str join ', ') has no native ($feature); every result must measure it with the control it is scaled by"
        }
    }
    let measured = $reports
    | each {|report| $report.results | where feature == $feature and implementation == "native" }
    | flatten
    let median = {|rows: list<record>, get_value: closure|
        $rows | each $get_value | math median
    }
    let benchmark_of = {|id: string, rows: list<record>|
        let benchmarks = $rows | get --optional benchmark | uniq
        if ($benchmarks | length) != 1 or $benchmarks.0 == null {
            error make {
                msg: $"($id) was not measured by one fingerprinted version of the benchmark; measure a commit whose scripts/latency/measure.nu records `benchmark`"
            }
        }
        $benchmarks.0
    }
    let controls = $reports | each {|report|
        $report.results | where feature == $CONTROL and implementation == "native"
    }
    let control_ids = $controls | each { get id | sort } | uniq
    if ($control_ids | length) != 1 or ($control_ids.0 | is-empty) {
        error make {
            msg: $"every result must measure the same native ($CONTROL) scenarios; `parity` scales the baseline by them"
        }
    }
    let control_scenarios = $controls | flatten | group-by id | items {|id, rows|
        {
            id: $id
            benchmark: (do $benchmark_of $id $rows)
            p95_ms: (do $median $rows {|row| $row.p95_ms })
        }
    } | sort-by id
    let control = {
        scenarios: $control_scenarios
        p95_ms: ($control_scenarios | get p95_ms | math median)
    }
    let recorded = $measured | group-by id | items {|id, rows|
        {
            id: $id
            feature: $feature
            implementation: "native"
            commit: $commits.0
            benchmark: (do $benchmark_of $id $rows)
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
            control: $control
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
# `tolerance` percent of it or `floor`, whichever is larger, after a recorded baseline is
# multiplied by its control's `scale`. Without a baseline, `regressed` when it exceeds the
# candidate's own `budget_ms`, and `new` when it has none.
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
                    scale: null
                    candidate_p95_ms: ($cand.p95_ms | math round --precision 2)
                    allowed_ms: $budget
                    limit_ms: $budget
                    verdict: (
                        if $budget == null { "new" } else if $cand.p95_ms > $budget { "regressed" } else { "ok" }
                    )
                }
            }
            $base => {
                let scale = $pair.control?.scale?
                let held = $base.p95_ms * ($scale | default 1.0)
                let allowed = [($held * $tolerance / 100) ($floor / 1ms)] | math max
                {
                    id: $cand.id
                    baseline: $base.implementation
                    candidate: $cand.implementation
                    baseline_p95_ms: ($base.p95_ms | math round --precision 2)
                    scale: ($scale | if $in == null { null } else { math round --precision 3 })
                    candidate_p95_ms: ($cand.p95_ms | math round --precision 2)
                    allowed_ms: ($allowed | math round --precision 2)
                    limit_ms: ($held + $allowed | math round --precision 2)
                    verdict: (if $cand.p95_ms > $held + $allowed { "regressed" } else { "ok" })
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
        $"## ($title)\n\np95 keypress-to-results latency; a regression is more than ($tolerance)% or ($floor), whichever is larger. A baseline recorded at an earlier commit is first multiplied by `scale`, this run's native ($CONTROL) p95 over the one recorded with it; `limit_ms` is the most the candidate may take.\n\n($rows | to md)\n\n"
        | save --append $summary
    }
    let regressed = $rows | where verdict == "regressed"
    if not $report and ($regressed | is-not-empty) {
        error make {
            msg: $"p95 latency regressed beyond tolerance: ($regressed | get id | str join ', ')"
        }
    }
}

const MARKER = "<!-- dupin-latency -->"
const NAME = '^[a-z0-9][a-z0-9.-]{0,63}$'
const SHA = '^[0-9a-f]{40}$'

# Download the latency results of the main build `commit` is compared with
# (scripts/main-build.nu) to `--out`, with the CI run they came from. Writes nothing when no main
# build has any.
def "main fetch-main" [
    repo: string # The GitHub repository whose main builds to look in
    commit: string # The commit measured; on a pull request, its merge commit
    --branch: string = "main"
    --out: path = "target/latency/main.json"
]: nothing -> nothing {
    let found = find-main-artifact $repo $commit $branch latency results.json {|file|
        open $file | get --optional results | checked-results
    }
    if $found == null {
        print --stderr "no main build has latency results"
        return
    }
    mkdir ($out | path dirname)
    {
        schema: 1
        run: {
            id: $found.id
            commit: $found.commit
            merge_base: $found.merge_base
            base: $found.base
        }
        results: $found.data
    }
    | to json
    | save --force $out
    print --stderr $"wrote main's results from ($found.url) to ($out)"
}

# Render the results in `results` against main's as Markdown, the Latency comment workflow's
# comment. A scenario is flagged as `compare` would flag it, after main's figures are scaled by
# the native control; nothing fails on it.
def "main report" [
    results: path # `run`'s output
    main?: path # `fetch-main`'s output; without it there is nothing to compare with
    --repo: string = "" # The GitHub repository, to link main's run
    --commit: string = "" # The pull request's head, to name in the heading
    --run-url: string = "" # The run that measured `results`
]: nothing -> string {
    let measured = open $results | get --optional results | checked-results
    let main = if $main == null {
        null
    } else {
        open $main | checked-main
    }
    let compared = (against-main
        $measured
        ($main | get --optional results)
        $COMPARE_TOLERANCE
        $COMPARE_FLOOR
    )
    (render
        $compared
        ($main | get --optional run)
        {repo: $repo, commit: $commit, run_url: $run_url}
    )
}

# The fields of `run`'s results that `report` reads, each checked. The report reads a pull
# request's results with a token that can comment, so every name it prints must be one a
# scenario could have and every figure a number.
export def checked-results []: any -> list<record> {
    let results = $in
    if not (($results | describe) =~ '^(list|table)') {
        error make {msg: "the results are not a list"}
    }
    let ms = {|value, what: string|
        if ($value | describe) not-in [int float] or $value < 0 or $value > 60000 {
            error make {msg: $"($what) is not a time in milliseconds"}
        }
        $value | into float
    }
    let checked = $results | each {|result|
        if not ($result | describe | str starts-with record) {
            error make {msg: "a result is not a record"}
        }
        let name = {|key: string|
            let value = $result | get --optional $key
            if ($value | describe) != string or not ($value =~ $NAME) {
                error make {msg: $"a result's ($key) is not a scenario's name"}
            }
            $value
        }
        let id = do $name id
        let stages = $result | get --optional stages | default {}
        if not ($stages | describe | str starts-with record) {
            error make {msg: $"($id)'s stages are not a record"}
        }
        {
            id: $id
            feature: (do $name feature)
            implementation: (do $name implementation)
            p95_ms: (do $ms ($result | get --optional p95_ms) $"($id)'s p95_ms")
            stages: (
                stage-names
                | each {|stage|
                    let figures = $stages | get --optional $stage
                    if not ($figures | describe | str starts-with record) {
                        error make {msg: $"($id)'s ($stage) is not a record"}
                    }
                    {$stage: {p95: (do $ms ($figures | get --optional p95) $"($id)'s ($stage)")}}
                }
                | into record
            )
        }
    }
    let keys = $checked | each {|result| $"($result.id) [($result.implementation)]" }
    if ($keys | uniq | length) != ($keys | length) {
        error make {msg: "a scenario is listed twice for one implementation"}
    }
    $checked
}

# `fetch-main`'s output, checked as `checked-results` checks a run's.
export def checked-main []: any -> record {
    let main = $in
    let run = $main | get --optional run | default {}
    let id = $run | get --optional id
    if ($id | describe) != int or $id <= 0 {
        error make {msg: "main's run id is not a number"}
    }
    let merge_base = $run | get --optional merge_base
    if ($merge_base | describe) != bool {
        error make {msg: "whether main's run is at the merge base is not a boolean"}
    }
    let sha = {|value, what: string, empty: bool|
        if ($value | describe) != string or not ($value =~ $SHA or ($empty and $value == "")) {
            error make {msg: $"($what) is not a full SHA"}
        }
        $value
    }
    {
        run: {
            id: $id
            commit: ((do
                $sha
                ($run | get --optional commit)
                "main's commit"
                false
            ))
            merge_base: $merge_base
            base: ((do
                $sha
                ($run | get --optional base)
                "the merge base"
                true
            ))
        }
        results: ($main | get --optional results | checked-results)
    }
}

# Each scenario of `results` against main's, held as `compare` holds it, but with main's p95
# first multiplied by `scale`: this run's median native CONTROL p95 over main's, as `parity`
# scales a recorded baseline, so a slower runner does not read as a slower pull request. The
# control scenarios are held to main's unscaled, since the scale is made of them. Nothing is
# scaled when the two did not measure the same control scenarios.
export def against-main [
    results: list<record>
    main
    tolerance: float
    floor: duration
]: nothing -> record {
    let main = $main | default []
    let is_control = {|result| $result.feature == $CONTROL and $result.implementation == "native" }
    let ours = $results | where {|result| do $is_control $result }
    let theirs = $main | where {|result| do $is_control $result }
    let scale = if ($ours | is-empty) or ($ours | get id | sort) != ($theirs | get id | sort) {
        null
    } else {
        let was = $theirs | get p95_ms | math median
        if $was > 0 { ($ours | get p95_ms | math median) / $was } else { null }
    }
    let row = {|result: record, control: bool|
        {
            id: $result.id
            implementation: $result.implementation
            control: $control
            scale: null
            main_p95_ms: null
            held_p95_ms: null
            pr_p95_ms: null
            delta_ms: null
            delta_percent: null
            limit_ms: null
            verdict: null
            stages: []
        }
    }
    let compared = $results | each {|result|
        let control = do $is_control $result
        let base = $main | where id == $result.id and implementation == $result.implementation | get 0?
        let factor = if $control { null } else { $scale }
        if $base == null {
            do $row $result $control | merge {pr_p95_ms: $result.p95_ms, verdict: "new"}
        } else {
            let verdict = verdicts [
                {
                    candidate: $result
                    baseline: $base
                    control: (if $factor == null { null } else { {scale: $factor} })
                }
            ] $tolerance $floor
            | first
            let times = $factor | default 1.0
            let held = $base.p95_ms * $times
            do $row $result $control | merge {
                scale: $factor
                main_p95_ms: $base.p95_ms
                held_p95_ms: $held
                pr_p95_ms: $result.p95_ms
                delta_ms: ($result.p95_ms - $held)
                delta_percent: (if $held > 0 { ($result.p95_ms - $held) * 100 / $held } else { null })
                limit_ms: $verdict.limit_ms
                verdict: $verdict.verdict
                stages: (
                    stage-names | each {|stage|
                        let was = ($base.stages | get $stage | get p95) * $times
                        let now = $result.stages | get $stage | get p95
                        {stage: $stage, main_ms: $was, pr_ms: $now, delta_ms: ($now - $was)}
                    }
                )
            }
        }
    }
    let removed = $main
    | where {|base| $results | where id == $base.id and implementation == $base.implementation | is-empty }
    | each {|base|
        do $row $base (do $is_control $base) | merge {main_p95_ms: $base.p95_ms, verdict: "removed"}
    }
    {
        scale: $scale
        tolerance: $tolerance
        floor_ms: ($floor / 1ms)
        rows: ($compared ++ $removed)
    }
}

# The comment's Markdown. `compared` is `against-main`'s record, `main` the run main's results
# came from, or nothing, and `context` the repository, the pull request's head and the run that
# measured it.
export def render [compared: record, main, context: record]: nothing -> string {
    let short = {|commit: string|
        $commit | str substring 0..6
    }
    let regressed = $compared.rows | where verdict == "regressed"
    let heading = if ($context.commit? | default "") == "" { "### Keypress latency" } else {
        $"### Keypress latency on (do $short $context.commit)"
    }
    let tolerance = $"(plain $compared.tolerance)% or (plain $compared.floor_ms) ms, whichever is larger"
    let main_link = if $main == null { "main" } else if ($context.repo? | default "") == "" {
        $"main at (do $short $main.commit)"
    } else {
        $"[main at (do $short $main.commit)]\(https://github.com/($context.repo)/actions/runs/($main.id)\)"
    }
    let against = if $main == null {
        "No main build has measured latency yet, so there is nothing to compare with."
    } else if $main.merge_base {
        $"Against ($main_link), which this pull request merged onto."
    } else if $main.base != "" {
        $"Against ($main_link), the latest main build measured; main at (do $short $main.base), which this pull request merged onto, has no measurement."
    } else {
        $"Against ($main_link), the latest main build measured."
    }
    let scaling = if $compared.scale == null {
        $"This run and main's did not both measure the native `($CONTROL)` scenarios, so nothing is scaled and a slower runner reads as a slower pull request."
    } else {
        $"Main's figures are multiplied by ×(fixed $compared.scale 3), this run's median native `($CONTROL)` p95 over main's, so the runner's speed cancels out; the `($CONTROL)` scenarios themselves are compared unscaled."
    }
    let table = [
        "| Scenario | main | main, scaled | this PR | change | limit | |"
        "| :-- | --: | --: | --: | --: | --: | :-- |"
        ...($compared.rows | each {|row| scenario-row $row })
    ] | str join "\n"
    let run = if ($context.run_url? | default "") == "" { "" } else { $" Measured by [this run]\(($context.run_url)\)." }
    let every = [
        $"<details><summary>Every scenario</summary>\n"
        $against
        $scaling
        $"A scenario is flagged when its p95 exceeds main's, scaled, by more than ($tolerance); `limit` is the most it may take.($run)\n"
        $table
        "\n</details>"
    ] | str join "\n"
    let summary = $"<details><summary>Machine-readable summary</summary>\n\n```json\n(machine-summary $compared $main $context.commit? | to json --indent 2)\n```\n\n</details>"
    let body = if $main == null {
        [$heading $against $every]
    } else if ($regressed | is-empty) {
        [
            $heading
            $"No scenario is slower than ($main_link) beyond ($tolerance), after scaling by the native `($CONTROL)` control (scale-text $compared.scale)."
            $every
            $summary
        ]
    } else {
        let names = $regressed | each {|row| $"`($row.id)`" } | str join ", "
        let count = $regressed | length
        [
            $heading
            $":warning: **($count) scenario(if $count == 1 { ' is' } else { 's are' }) slower than main after scaling, beyond ($tolerance): ($names).** Latency never blocks merging; this comment is for whoever works on or reviews this pull request."
            $"($against) ($scaling)"
            (what-to-do $regressed)
            (stage-tables $regressed $compared.scale)
            $every
            $summary
        ]
    }
    $"($MARKER)\n($body | str join "\n\n")\n"
}

def scale-text [scale]: nothing -> string {
    if $scale == null { "; unavailable, so unscaled" } else { $"(char lparen)×(fixed $scale 3)(char rparen)" }
}

def plain [value: number]: nothing -> string {
    if $value == ($value | math round) {
        $value | into int | into string
    } else {
        $value | into string
    }
}

def fixed [value: number, decimals: int]: nothing -> string {
    $value | into float | math round --precision $decimals | into string --decimals $decimals
}

def ms [value]: nothing -> string {
    if $value == null { "–" } else { $"(fixed $value 2) ms" }
}

def signed [value: number, decimals: int]: nothing -> string {
    let rounded = $value | into float | math round --precision $decimals
    $"(if $rounded < 0 { '-' } else { '+' })(fixed ($rounded | math abs) $decimals)"
}

def change [row: record]: nothing -> string {
    if $row.delta_ms == null {
        return "–"
    }
    let percent = if $row.delta_percent == null { "" } else { $" \((signed $row.delta_percent 1)%\)" }
    $"(signed $row.delta_ms 2) ms($percent)"
}

def scenario-row [row: record]: nothing -> string {
    let label = if $row.control { $"`($row.id)` \(($row.implementation), control\)" } else {
        $"`($row.id)` \(($row.implementation)\)"
    }
    let mark = match $row.verdict {
        regressed => ":warning: slower"
        new => "new"
        removed => "removed"
        _ => ""
    }
    $"| ($label) | (ms $row.main_p95_ms) | (ms $row.held_p95_ms) | (ms $row.pr_p95_ms) | (change $row) | (ms $row.limit_ms) | ($mark) |"
}

def largest-stage [row: record]: nothing -> any {
    $row.stages | sort-by delta_ms | last | if $in == null or $in.delta_ms <= 0 { null } else { $in }
}

def what-to-do [regressed: list<record>]: nothing -> string {
    let findings = $regressed | each {|row|
        let scaled = if $row.control { "than main (unscaled, as the control)" } else { "than main after scaling" }
        let stage = largest-stage $row
        let share = if $stage == null { "" } else {
            $" The largest share is in `($stage.stage)` \((signed $stage.delta_ms 2) ms\)."
        }
        $"- `($row.id)` \(($row.implementation)\) p95 is (fixed $row.delta_ms 2) ms \((fixed $row.delta_percent 1)%\) slower ($scaled): (ms $row.pr_p95_ms) against a limit of (ms $row.limit_ms).($share)"
    }
    let control = if ($regressed | any {|row| $row.control }) {
        [
            ""
            $"The native `($CONTROL)` scenarios are the control the others are scaled by, and they are slower themselves. That is either a slower runner or a change every keypress pays for; in the second case, scaling hides the same slowdown in the other scenarios, so compare their unscaled `main` column too."
        ]
    } else { [] }
    let only = $regressed | first | get id | split row "." | first
    [
        "#### What to do"
        ""
        ...$findings
        ...$control
        ""
        "1. Look at what this pull request changes on the per-keystroke path, starting with the stage named above: `dispatch_ms` hands the query's searches to the providers' workers, `provider_ms` is the providers' own searching, `merge_ms` merges and ranks their results on the UI thread (including waiting for it), and `layout_ms` builds, lays out and encodes the next frame."
        "2. Re-run the `latency` job: shared runners are noisy, and a slowdown that does not come back was noise."
        $"3. Measure main and this branch on one machine: on each, `cargo build --profile latency -p dupin` and `nu scripts/latency.nu run --only ($only) --out target/latency/<main or branch>.json`, then `nu scripts/latency.nu compare target/latency/main.json target/latency/branch.json`."
        "4. If the slowdown is intended, say why in the pull request."
    ] | str join "\n"
}

def stage-tables [regressed: list<record>, scale]: nothing -> string {
    let note = if $scale == null { "" } else { ", main's multiplied by the scale like the total (the control's unscaled)" }
    let tables = $regressed | each {|row|
        let rows = $row.stages | each {|stage|
            $"| `($stage.stage)` | (ms $stage.main_ms) | (ms $stage.pr_ms) | (signed $stage.delta_ms 2) ms |"
        }
        [
            $"**`($row.id)`** \(($row.implementation)\)"
            ""
            "| stage | main | this PR | change |"
            "| :-- | --: | --: | --: |"
            ...$rows
        ] | str join "\n"
    }
    [
        "#### Where the time went"
        $"The p95 of each stage of the slower scenarios($note). Each stage's p95 is its own, so they do not add up to the total's."
        ...$tables
    ] | str join "\n\n"
}

# The comparison as data, for an agent reading the comment: every flagged scenario with its
# figures and stages, in milliseconds.
export def machine-summary [compared: record, main, commit]: nothing -> record {
    let round = {|value|
        if $value == null {
            null
        } else {
            $value | math round --precision 3
        }
    }
    {
        schema: 1
        commit: ($commit | default "")
        main: (
            if $main == null { null } else { {commit: $main.commit, run_id: $main.id, merge_base: $main.merge_base} }
        )
        control: $CONTROL
        scale: (do $round $compared.scale)
        tolerance_percent: $compared.tolerance
        floor_ms: $compared.floor_ms
        regressed: (
            $compared.rows | where verdict == "regressed" | each {|row|
                {
                    id: $row.id
                    implementation: $row.implementation
                    control: $row.control
                    scaled: ($row.scale != null)
                    main_p95_ms: (do $round $row.main_p95_ms)
                    scaled_main_p95_ms: (do $round $row.held_p95_ms)
                    pr_p95_ms: (do $round $row.pr_p95_ms)
                    limit_ms: $row.limit_ms
                    delta_ms: (do $round $row.delta_ms)
                    delta_percent: (do $round $row.delta_percent)
                    largest_stage: (largest-stage $row | get --optional stage)
                    stages: (
                        $row.stages
                        | each {|stage|
                            {
                                $stage.stage: {
                                    main_ms: (do $round $stage.main_ms)
                                    pr_ms: (do $round $stage.pr_ms)
                                    delta_ms: (do $round $stage.delta_ms)
                                }
                            }
                        }
                        | into record
                    )
                }
            }
        )
    }
}
