export def record [out: path, scenario: closure] {
    let raw = $out | path join recording
    mkdir $raw
    let source = $raw | path join launcher-flow.mov
    let movie = $out | path join launcher-flow.mp4
    # Let macOS finish the movie; killing capture can leave it unreadable.
    job spawn {
        let result = ^screencapture -x -v -V 45 -T 0 -D 1 -k $source | complete
        $result.stdout | save --force ($raw | path join capture.stdout)
        $result.stderr | save --force ($raw | path join capture.stderr)
        $result.exit_code | job send 0 --tag 45
    } | ignore
    sleep 1sec
    let scenario_error = try {
        do $scenario
        null
    } catch {|error|
        $error
    }
    let recording_error = try {
        let status = job recv --tag 45 --timeout 60sec
        if $status != 0 { error make {msg: 'Screen recording failed; see recording/capture.stderr'} }
        let result = ^avconvert -s $source -o $movie -p Preset1920x1080 | complete
        $result.stdout | save --force ($raw | path join conversion.stdout)
        $result.stderr | save --force ($raw | path join conversion.stderr)
        if $result.exit_code != 0 {
            if ($movie | path exists) { rm $movie }
            error make {
                msg: 'Video conversion failed; raw recording and logs are preserved'
            }
        }
        null
    } catch {|error| $error }
    if $scenario_error != null { error make $scenario_error }
    if $recording_error != null { error make $recording_error }
}
