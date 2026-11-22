#!/usr/bin/env nu

def --wrapped main [...args: string] {
    match ($env.CURRENT_FILE | path basename) {
        screencapture => {
            if ($args | take 9) != [
                -x
                -v
                -V
                '45'
                -T
                '0'
                -D
                '1'
                -k
            ] { exit 20 }
            sleep 1500ms
            'capture-finished' | save --append $env.VIDEO_TRACE
            if $env.VIDEO_FAILURE == capture { exit 21 }
            'fixture movie' | save ($args | last)
        }
        avconvert => {
            if not (open --raw $env.VIDEO_TRACE | str contains capture-finished) { exit 22 }
            if $args.0 != '-s' or $args.2 != '-o' or ($args | last) != Preset1920x1080 { exit 23 }
            'fixture mp4' | save $args.3
            'converted' | save --append $env.VIDEO_TRACE
            if $env.VIDEO_FAILURE == conversion { exit 24 }
        }
    }
}
