export def record [out: path, scenario: closure] {
    let logs = $out | path join recording
    mkdir $logs
    let movie = $logs | path join launcher-flow.mp4
    let progress = $logs | path join capture.progress
    let errors = $logs | path join capture.stderr
    job spawn {
        let status = try {
            (^timeout --signal=INT --kill-after=5s 60s
                ffmpeg -nostdin -hide_banner -y
                -f avfoundation -framerate 15 -capture_cursor 1 -capture_mouse_clicks 1
                -i 'Capture screen 0:none' -t 45 -an
                -vf 'scale=1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2'
                -c:v libx264 -preset veryfast -crf 23 -pix_fmt yuv420p
                -movflags +faststart -map_metadata -1 -progress pipe:1 $movie
                o> $progress e> $errors)
            $env.LAST_EXIT_CODE
        } catch {|error|
            $error.msg | save --append $errors
            1
        }
        $status | job send 0 --tag 1
    } | ignore
    let scenario_error = try {
        wait-for-frame $progress
        do $scenario
        null
    } catch {|error| $error }
    let recording_error = try {
        let status = job recv --tag 1 --timeout 75sec
        if $status != 0 { error make {msg: 'Screen recording failed; see recording/capture.stderr'} }
        let result = ^ffprobe -v error -select_streams v:0 -show_entries stream=codec_name,nb_frames:format=duration -of json $movie | complete
        if $result.exit_code != 0 { error make {msg: 'Could not inspect the recording'} }
        let metadata = $result.stdout | from json
        $metadata | to json | save --force ($logs | path join metadata.json)
        let video = $metadata.streams | first
        if $video.codec_name != h264 or ($video.nb_frames | into int) < 600 or ($metadata.format.duration | into float) < 40 {
            error make {
                msg: 'Recording is too short or not H.264; see recording/metadata.json'
            }
        }
        mv $movie ($out | path join launcher-flow.mp4)
        null
    } catch {|error| $error }
    if $scenario_error != null {
        if $recording_error != null { print --stderr $recording_error.msg }
        error make $scenario_error
    }
    if $recording_error != null { error make $recording_error }
}

def wait-for-frame [progress: path] {
    for _ in 1..150 {
        if ($progress | path exists) and ((open --raw $progress) =~ '(?m)^frame=[1-9]') { return }
        sleep 100ms
    }
    error make {
        msg: 'Recording did not produce its first frame; see recording/capture.stderr'
    }
}
