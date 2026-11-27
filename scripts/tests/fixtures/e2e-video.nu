#!/usr/bin/env nu

def --wrapped main [...args: string] {
    match ($env.CURRENT_FILE | path basename) {
        timeout => {
            if ($args | take 3) != [--signal=INT --kill-after=5s 165s] { exit 20 }
            run-external ...($args | skip 3)
        }
        ffmpeg => {
            if $env.VIDEO_DELAY == '1' { sleep 500ms }
            if '-nostdin' not-in $args or 'Capture screen 0:none' not-in $args or '150' not-in $args { exit 21 }
            'capture-started' | save --append $env.VIDEO_TRACE
            print 'frame=1'
            sleep 1500ms
            'capture-finished' | save --append $env.VIDEO_TRACE
            if $env.VIDEO_FAILURE == capture { exit 22 }
            'fixture mp4' | save ($args | last)
        }
        ffprobe => {
            if not (open --raw $env.VIDEO_TRACE | str contains capture-finished) { exit 23 }
            'inspected' | save --append $env.VIDEO_TRACE
            if $env.VIDEO_FAILURE == inspection { exit 24 }
            {
                streams: [
                    {
                        codec_name: (
                            if $env.VIDEO_FAILURE == codec { 'hevc' } else { 'h264' }
                        )
                        nb_frames: (if $env.VIDEO_FAILURE == frames { '1' } else { '675' })
                    }
                ]
                format: {
                    duration: (if $env.VIDEO_FAILURE == short { '1' } else { '45' })
                }
            } | to json | print
        }
    }
}
