#!/usr/bin/env nu

use headless-config.nu scenario-env

# Render each headless scenario in scripts/headless and fail if any run's peak physical footprint
# (what Activity Monitor shows as "Memory" at its highest) goes over `--budget`.
#
# Most of the peak belongs to the GPU driver, not Dupin: see "Peak memory" in
# docs/development.md. That share scales with the Mac's GPU, so the default budget fits the
# M5 Max it was measured on; pass `--budget` elsewhere.
def main [
    --bin: path = "target/debug/dupin" # A debug build, or a release build with Rune's `tooling` feature
    --budget: filesize = 450MB # The highest peak footprint a scenario may reach
]: nothing -> table {
    let root = $env.FILE_PWD | path dirname
    cd $root
    let tmp = mktemp --directory
    let runs = glob scripts/headless/*.json | sort | each {|script|
        let name = $script | path parse | get stem
        let png = $tmp | path join $"($name).png"
        let peak = with-env (scenario-env $script $tmp) {
            peak-footprint $bin --script $script --realtime --snap $png --at mark:end
        }
        {name: $name, peak: $peak}
    }
    rm --recursive $tmp
    let over = $runs | where peak > $budget
    if ($over | is-not-empty) {
        error make {msg: $"peak footprint over the ($budget) budget:\n($over | table)"}
    }
    $runs
}

# Run Dupin headless and return its peak physical footprint, as `/usr/bin/time -l` reports it.
def --wrapped peak-footprint [bin: path, ...args: string]: nothing -> filesize {
    let result = ^/usr/bin/time -l $bin ...$args | complete
    if $result.exit_code != 0 {
        error make {msg: $"($bin) ($args | str join ' ') failed:\n($result.stderr)"}
    }
    let peak = $result.stderr | parse --regex '(?m)^\s*(?<bytes>\d+)\s+peak memory footprint$'
    if ($peak | is-empty) {
        error make {
            msg: $"/usr/bin/time reported no peak memory footprint:\n($result.stderr)"
        }
    }
    $peak | first | get bytes | into int | into filesize
}
