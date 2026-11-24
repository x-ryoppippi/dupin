# How scripts/latency.nu measures a scenario. Everything that decides what a recorded native
# baseline means lives here, so `fingerprint` can tell a baseline measured by another version
# of the benchmark from one this version would measure alike.

const HARNESS = path self

# The clipboard history every implementation is handed, whatever its own config.
const CLIPBOARD = path self ../fixtures/clipboard.json

# Measure every scenario of `catalogue` with every implementation its feature declares, from
# the repository root. Each result carries the `fingerprint` of the benchmark it was measured by.
export def measure-all [
    catalogue: record
    bin: path
    settings: record
    --only: string
    --implementation: string
]: nothing -> list<record> {
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
    let tmp = mktemp --directory
    # Round by round rather than scenario by scenario, so whatever else the machine is doing
    # weighs on every implementation alike.
    let measured = 1..$settings.runs | each {|run|
        $jobs | enumerate | each {|job|
            let name = $"($job.item.id) [($job.item.implementation)]"
            print --stderr $"run ($run)/($settings.runs): ($name)"
            let dir = $tmp | path join $"($job.item.id)-($job.item.implementation)-($run)"
            {job: $job.index, run: (measure $bin $job.item $settings $dir)}
        }
    } | flatten
    rm --recursive $tmp
    $jobs | enumerate | each {|job|
        summarize $job.item ($measured | where job == $job.index | get run)
        | insert benchmark (fingerprint $job.item $settings)
    }
}

# What a scenario's measurement depends on besides the implementation under test: this
# harness, the fixtures every implementation shares, the keys typed and the run's settings.
# Two results with the same fingerprint were measured alike, so a port can be held to a native
# result recorded at another commit only when theirs match.
export def fingerprint [scenario: record, settings: record]: nothing -> string {
    {
        # Comments say why, not what is measured; rewording one must not void every baseline.
        harness: (
            open --raw $HARNESS
            | lines
            | where {|line| not ($line | str trim | str starts-with "#") }
            | str join "\n"
            | hash sha256
        )
        clipboard: (open --raw $CLIPBOARD | hash sha256)
        prefix: $scenario.prefix
        typed: $scenario.typed
        settings: ($settings | select samples warmup runs gap_ms fps)
    }
    | to json --raw
    | hash sha256
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
        DUPIN_HEADLESS_CLIPBOARD: $CLIPBOARD
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
    let package = $job.setup.package?
    let missed = unanswered $matched $provider $per_pass $package
    if $missed > 0 {
        let rows = if $package == null { "its rows" } else { $"rows from ($package)" }
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

# How many drawn keypresses, `per_pass` to a pass, did not search `provider`, or ended a pass
# without a row from it, or, given an extension's `package`, without rows that package answered.
# Only the whole of `typed` must show the feature's rows: a shorter query may rightly have none.
export def unanswered [
    samples: list<record>
    provider: string
    per_pass: int
    package?: string
]: nothing -> int {
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
        let answered = $package == null or ($it.item.packages? | default {} | get --optional $package | default 0) > 0
        let ends_pass = ($it.index + 1) mod $per_pass == 0
        not $searched or ($ends_pass and not ($shown and $answered))
    }
    | length
}

export def stage-names []: nothing -> list<string> {
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
