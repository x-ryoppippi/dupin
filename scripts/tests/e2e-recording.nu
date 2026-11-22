#!/usr/bin/env nu

use ../e2e-recording.nu record

def assert [condition: bool, message: string] {
    if not $condition { error make {msg: $message} }
}

let root = mktemp --directory | path expand

let failure = try {
    let bin = $root | path join bin
    mkdir $bin
    for name in [screencapture avconvert] {
        let destination = $bin | path join $name
        cp scripts/tests/fixtures/e2e-video.nu $destination
        ^chmod +x $destination
    }
    for kind in [none scenario capture conversion] {
        let out = $root | path join $kind
        mkdir $out
        let trace = $out | path join trace
        '' | save $trace
        let problem = with-env {
            PATH: ($env.PATH | prepend $bin)
            VIDEO_TRACE: $trace
            VIDEO_FAILURE: $kind
        } {
            try {
                record $out {
                    'scenario-ran' | save --append $trace
                    if $kind == scenario { error make {msg: 'scenario fixture failed'} }
                }
                null
            } catch {|error| $error.msg }
        }
        let movie = $out | path join launcher-flow.mp4
        assert ((open --raw $trace | str contains capture-finished)) 'Capture must finish even on a scenario error'
        assert (($movie | path exists) == ($kind in [none scenario])) 'Only complete conversions may be attached'
        if $kind == none {
            assert ($problem == null) 'Successful recording must succeed'
        } else {
            assert ($problem != null) 'Failures must be reported'
        }
        if $kind == scenario {
            assert ($problem == 'scenario fixture failed') 'Recording must preserve the scenario failure'
        }
    }
    null
} catch {|error| $error }
rm --recursive $root
if $failure != null { error make $failure }
print 'e2e-recording: success, scenario failure, capture failure and conversion failure passed'
