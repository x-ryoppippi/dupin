#!/usr/bin/env nu

use ../e2e-recording.nu record

def assert [condition: bool, message: string] {
    if not $condition { error make {msg: $message} }
}

let root = mktemp --directory | path expand

let failure = try {
    let bin = $root | path join bin
    mkdir $bin
    for name in [timeout ffmpeg ffprobe] {
        let destination = $bin | path join $name
        cp scripts/tests/fixtures/e2e-video.nu $destination
        ^chmod +x $destination
    }
    for kind in [
        none
        scenario
        capture
        inspection
        short
        codec
        frames
    ] {
        let out = $root | path join $kind
        mkdir $out
        let trace = $out | path join trace
        '' | save $trace
        let problem = with-env {
            PATH: ($env.PATH | prepend $bin)
            VIDEO_TRACE: $trace
            VIDEO_FAILURE: $kind
        } {
            try { ^sh -c 'exit 3' } catch { }
            try {
                record $out {
                    assert ((open --raw $trace | str contains capture-started)) 'Capture must start before the scenario'
                    'scenario-ran' | save --append $trace
                    if $kind == scenario { error make {msg: 'scenario fixture failed'} }
                }
                null
            } catch {|error| $error.msg }
        }
        let movie = $out | path join launcher-flow.mp4
        assert ((open --raw $trace | str contains scenario-ran)) 'The scenario must run once capture starts'
        assert ((open --raw $trace | str contains capture-finished)) 'Capture must finish even on a scenario error'
        assert (($movie | path exists) == ($kind in [none scenario])) $'Only validated movies may be attached: ($kind): ($problem)'
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
print 'e2e-recording: lifecycle, first-frame ordering and movie validation passed'
