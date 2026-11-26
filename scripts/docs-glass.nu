#!/usr/bin/env nu

use docs-images.nu [
    MANIFEST
    GLASS
    CLIP_FPS
    CAPTION_SPACE
    manifest-entries
    image-id
    image-themes
    theme-path
    poster-path
    clip-timeline
    caption
    encode-video
    scratch-environment
]
use e2e/keys.nu key-script

const STAGE = path self docs-glass/Stage.swift
# Wallpaper around the panel, in points, so the glass's edge shows against it.
const MARGIN = 32

# The wallpapers the glass is recorded over, drawn here rather than shipped: soft blobs of colour
# for the glass to bend, with nothing sharp to read through the text. Positions and radii are in
# a 64×40 grid.
const WALLPAPERS = {
    dusk: {
        base: "#1d1747"
        blobs: [
            [x, y, r, colour];
            [14, 10, 16, "#c3418f"]
            [50, 31, 15, "#f08a3c"]
            [52, 6, 12, "#3a6df0"]
            [30, 36, 10, "#7b3fd0"]
        ]
    }
    lagoon: {
        base: "#08263a"
        blobs: [
            [x, y, r, colour];
            [12, 30, 16, "#1fb7a6"]
            [48, 10, 15, "#2f7df6"]
            [56, 34, 10, "#8fe07a"]
            [28, 6, 9, "#5ad1f0"]
        ]
    }
    ember: {
        base: "#2a0f12"
        blobs: [
            [x, y, r, colour];
            [16, 12, 15, "#e2502f"]
            [50, 30, 16, "#f2b33d"]
            [46, 4, 10, "#b8306a"]
            [26, 36, 10, "#ff8a4c"]
        ]
    }
}

# Record the manifest's `glass = true` clips from the real app over Liquid Glass, which a
# headless render can't draw, to the files `just docs-clips` renders in their place, and list
# them in docs/images/glass.json.
#
# Dupin runs as its own instance with a scratch home and configuration, as the documentation
# images do, behind which a borderless window shows the wallpaper: the desktop picture itself is
# never changed. The clip's steps are pressed through System Events and the screen is recorded
# with ffmpeg, so the terminal running this needs Accessibility and Screen Recording, which this
# never asks for. The panel follows the system appearance, and the clips are dark, so on a Mac in
# Light mode it stops, and on CI it switches the runner to Dark.
def main [
    --app: path = "result/Applications/Dupin.app" # The app bundle to record
    --only: string # Record only the clips whose `<page>/<name>` contains this
    --wallpaper: string # A preset (dusk, lagoon or ember) or an image, for every clip
]: nothing -> table {
    let root = $env.FILE_PWD | path dirname
    cd $root
    let binary = $app | path expand | path join Contents MacOS dupin
    if not ($binary | path exists) {
        error make {
            msg: $"($binary) does not exist; `just build` makes result/Applications/Dupin.app"
        }
    }
    let macos = ^sw_vers -productVersion | str trim
    if ($macos | split row . | first | into int) < 26 {
        error make {msg: $"Liquid Glass needs macOS 26 or later; this Mac runs ($macos)"}
    }
    let manifest = open $MANIFEST
    let clips = manifest-entries $manifest [clips]
    | where {|clip| ($clip.glass? | default false) and ($only == null or (image-id $clip | str contains $only)) }
    if ($clips | is-empty) {
        error make {
            msg: $"no `glass = true` clip in ($MANIFEST | path basename) matches ($only)"
        }
    }
    # Each instance's log, scratch home and raw recording, kept until the next run.
    let tmp = "target/docs-glass" | path expand
    rm --recursive --force $tmp
    mkdir $tmp
    let stage = $tmp | path join stage
    # The system's Swift with the system SDK, not the dev shell's Nix one, which has no Swift.
    ^env -u SDKROOT -u DEVELOPER_DIR /usr/bin/swiftc -O -framework AppKit $STAGE -o $stage
    if (^$stage preflight | complete).exit_code != 0 {
        error make {
            msg: "this terminal may not record the screen: allow it in System Settings → Privacy & Security → Screen Recording, then run this again"
        }
    }
    dark-mode
    let retina = if ($env.CI? | default "") == "true" { retina-display $stage }
    print --stderr (^$stage displays)
    print --stderr (
        ^ffmpeg -hide_banner -f avfoundation -list_devices true -i ""
        | complete
        | get stderr
    )
    let recorded = $clips | each {|clip|
        let chosen = $wallpaper | default ($clip.wallpaper? | default $manifest.wallpaper)
        record-clip $clip $manifest $binary $stage (wallpaper-image $chosen $tmp) $tmp
        | insert wallpaper $chosen
        | insert macos $macos
    }
    let captured = if ($GLASS | path exists) {
        open $GLASS | get clips
    } else {
        {}
    }
    let captured = $recorded | reduce --fold $captured {|clip, all|
        $all | upsert $clip.image ($clip | select macos wallpaper scale)
    }
    {clips: $captured} | to json --tabs 1 | $"($in)\n" | save --force $GLASS
    if $retina != null { job kill $retina }
    $recorded | select image bytes wallpaper scale
}

# A CI runner's display is a 1x framebuffer with no 2x mode, so its clips would be half the
# resolution of a Retina Mac's. This makes a 2x virtual display the main one, which needs no
# permission and lasts as long as its job, and returns that job, or null when there is none and
# the clips stay at 1x. Never on a Mac in use, whose displays it would rearrange.
def retina-display [stage: path]: nothing -> any {
    let job = job spawn { ^$stage hidpi }
    let ready = try {
        wait-for "the 2x display" { (^$stage scale | into float) == 2.0 }
        true
    } catch { false }
    if $ready { return $job }
    print --stderr "no 2x display: recording at 1x"
    job kill $job
    null
}

# The panel is drawn in the system appearance, and the clips are of the dark one.
def dark-mode [] {
    let style = ^defaults read -g AppleInterfaceStyle | complete
    if ($style.stdout | str trim) == Dark { return }
    if ($env.CI? | default "") != "true" {
        error make {
            msg: "switch this Mac to Dark in System Settings → Appearance; the clips show the dark panel"
        }
    }
    ^osascript -e 'tell application "System Events" to tell appearance preferences to set dark mode to true'
}

# The image a wallpaper names: a preset drawn at 2560×1600, or a file as it is.
def wallpaper-image [name: string, tmp: path]: nothing -> string {
    let preset = $WALLPAPERS | get --optional $name
    if $preset == null {
        if not ($name | path exists) {
            error make {
                msg: $"the wallpaper ($name) is neither a preset \(($WALLPAPERS | columns | str join ', ')\) nor a file"
            }
        }
        return ($name | path expand)
    }
    let out = $tmp | path join $"wallpaper-($name).png"
    if not ($out | path exists) {
        let blobs = $preset.blobs | each {|blob|
            [-fill $blob.colour -draw $"circle ($blob.x),($blob.y) ($blob.x + $blob.r),($blob.y)"]
        } | flatten
        ^magick -size 64x40 $"xc:($preset.base)" ...$blobs -blur 0x9 -resize 2560x1600! $out
    }
    $out
}

# One clip: Dupin over the wallpaper, its steps pressed in time, the screen recorded around the
# panel with the captions below it.
def record-clip [
    clip: record
    manifest: record
    binary: path
    stage: path
    wallpaper: path
    tmp: path
]: nothing -> record {
    let id = image-id $clip
    let name = $id | str replace --all / -
    print --stderr $"recording ($id)"
    let timeline = clip-timeline $clip.steps
    # Glass is the panel with no backdrop shader.
    let environment = scratch-environment ($clip.fixtures? | default ($tmp | path join none.json)) ($clip | reject --optional backdrop) $manifest $tmp
    let config = $environment.XDG_CONFIG_HOME | path join dupin config.toml
    # No hotkey, so it never takes the one a Dupin already running here holds: `dupin open`
    # shows its panel instead.
    open $config | upsert hotkey "" | upsert auto_update false | to toml | save --force $config
    let runtime = $tmp | path join $"($name)-runtime"
    mkdir $runtime
    ^chmod 700 $runtime
    # A launch with this scratch home lists only the fixture apps and keeps its history in a
    # scratch store, as a headless run does, so nothing of this Mac shows.
    let environment = $environment | merge {XDG_RUNTIME_DIR: $runtime, DUPIN_RECORDING: "1"}
    let log = $tmp | path join $"($name).log"
    let backdrop = job spawn { ^$stage wallpaper $wallpaper }
    let dupin = job spawn { with-env $environment { ^$binary o+e> $log } }
    let outcome = try {
        # Its control socket is there once it has started.
        wait-for $"($id) to start; see ($log)" {
            glob ($runtime | path join "**" "*") | is-not-empty
        }
        sleep 1sec
        let movie = $tmp | path join $"($name).mp4"
        let progress = $tmp | path join $"($name).progress"
        let seconds = ($timeline.events | last | get t) + 10
        job spawn {
            (
                ^ffmpeg -nostdin -hide_banner -loglevel error -y -f avfoundation -framerate 30
                -capture_cursor 0 -i "Capture screen 0:none" -t ($seconds | into string) -an
                -c:v libx264 -preset veryfast -crf 12 -pix_fmt yuv420p -stats_period 0.1
                -progress $progress $movie
            )
            true | job send 0 --tag 1
        } | ignore
        wait-for "the recording's first frame" { ($progress | path exists) and ((open --raw $progress) =~ '(?m)^out_time_us=[1-9]') }
        # When the recording began, from how far into it ffmpeg has written: the first progress
        # report comes some way in.
        let written = open --raw $progress | parse --regex '(?m)^out_time_us=(?P<us>\d+)' | last | get us | into int
        let started = (date now) - ($written * 1us)
        wait-for $"($id) to open; see ($log)" {
            (
                with-env $environment { ^$binary open | complete }
            ).exit_code == 0
        }
        let pid = dupin-pid $binary
        wait-for $"($id)'s panel to show; see ($log)" { (^$stage panel ($pid | into string) | complete).exit_code == 0 }
        # The window is listed a moment before the panel has faded in.
        sleep 1sec
        # The script touches this as it starts, which is when the clip's time 0 is.
        let mark = $tmp | path join $"($name).start"
        job spawn {
            ^osascript -e (steps-script $timeline.events $mark) | complete | ignore
            true | job send 0 --tag 2
        } | ignore
        # Where the panel is, as it grows and shrinks, until the last key.
        mut frames = []
        loop {
            let reply = ^$stage panel ($pid | into string) | complete
            if $reply.exit_code == 0 {
                $frames = $frames | append ($reply.stdout | from json)
            }
            if (try {
                job recv --tag 2 --timeout 200ms | ignore
                true
            } catch {
                false
            }) { break }
        }
        job recv --tag 1 --timeout 60sec | ignore
        if ($frames | is-empty) {
            error make {msg: $"($id): the panel never showed; see ($log)"}
        }
        let frame = $frames | skip 1 | reduce --fold ($frames | first) {|rect, all| union $all $rect }
        {
            clip: (
                (finish-glass
                    $clip
                    $timeline
                    $movie
                    $frame
                    (((ls $mark | get modified.0) - $started) / 1sec)
                    $tmp
                )
            )
        }
    } catch {|error| {error: $error} }
    job kill $dupin
    job kill $backdrop
    if $outcome.error? != null {
        error make $outcome.error.raw
    }
    $outcome.clip
}

# Cuts the recording to the panel and the wallpaper around it from when it was opened, draws the
# captions below it and encodes it where `just docs-clips` would have rendered it.
def finish-glass [
    clip: record
    timeline: record
    movie: path
    panel: record
    offset: float
    tmp: path
]: nothing -> record {
    let id = image-id $clip
    let name = $id | str replace --all / -
    let scale = $panel.scale
    let margin = $MARGIN * $scale | math round
    let space = $CAPTION_SPACE * $scale | math round
    let x = [
        ($panel.x - $margin)
        0
    ] | math max
    let y = [
        ($panel.y - $margin)
        0
    ] | math max
    # H.264 takes only even sides. The Dock and the menu bar are above the wallpaper, so the cut
    # stops short of them.
    let width = ($panel.width + 2 * $margin) // 2 * 2
    let height = ([
        ($panel.y + $panel.height + $margin + $space)
        $panel.bottom
    ] | math min) - $y | $in // 2 * 2
    let crop = $"crop=($width):($height):($x):($y)"
    # A key shows in the panel some way after System Events presses it, so the clip is lined up
    # on the frame the first key changed, not on when the script started.
    let first = $timeline.events | where type != mark | first | get t
    let changed = (first-change
        $movie
        $crop
        $offset
        ($first + 3)
        ($tmp | path join $"($name)-align")
    )
    let offset = if $changed == null { $offset } else { $changed - $first }
    let frames = $tmp | path join $"($name)-frames"
    mkdir $frames
    let duration = $timeline.events | last | get t
    (
        ^ffmpeg -nostdin -hide_banner -loglevel error -y -ss ($offset | into string) -i $movie
        -t ($duration | into string) -vf $"($crop),fps=($CLIP_FPS)"
        ($frames | path join frame_%05d.png)
    )
    let pngs = glob ($frames | path join frame_*.png) | sort
    caption $pngs $timeline.captions ($tmp | path join $"($name)-captions") $scale
    let files = image-themes $clip | each {|theme|
        let out = theme-path $clip $theme
        encode-video $pngs $out {poster: (poster-path $clip $theme), background: "#000000"}
        [$out (poster-path $clip $theme)]
    } | flatten
    {
        image: $id
        bytes: ($files | each { ls $in | get size.0 } | math sum)
        scale: $scale
    }
}

# How far into `movie`, from `from` and within `span` seconds, the `crop` first changes by more
# than the caret's blink, or null if it never does.
def first-change [
    movie: path
    crop: string
    from: float
    span: float
    folder: path
] {
    mkdir $folder
    (
        ^ffmpeg -nostdin -hide_banner -loglevel error -y -ss ($from | into string) -i $movie
        -t ($span | into string) -vf $"($crop),fps=30" ($folder | path join frame_%05d.png)
    )
    let pngs = glob ($folder | path join frame_*.png) | sort
    let base = $pngs | first
    let at = $pngs | enumerate | skip 1 | where {|frame|
        let compared = ^magick compare -metric AE -fuzz 8% $base $frame.item null: | complete
        ($compared.stderr | str trim | split row " " | first | into float) > 400
    } | get --optional 0.index
    if $at == null { null } else { $from + $at / 30 }
}

# The smallest rectangle holding both.
def union [a: record, b: record]: nothing -> record {
    let x = [$a.x $b.x] | math min
    let y = [$a.y $b.y] | math min
    let right = [
        ($a.x + $a.width)
        ($b.x + $b.width)
    ] | math max
    let bottom = [
        ($a.y + $a.height)
        ($b.y + $b.height)
    ] | math max
    {
        x: $x
        y: $y
        width: ($right - $x)
        height: ($bottom - $y)
        scale: $a.scale
        bottom: $a.bottom
    }
}

# The resident launcher's process, not a CLI or the extension host run from the same bundle.
def dupin-pid [binary: path]: nothing -> int {
    ^ps -axo pid=,command=
    | lines
    | parse --regex '^\s*(?P<pid>\d+) (?P<command>.*)$'
    | where command == $binary
    | get pid
    | first
    | into int
}

# The System Events script that plays a clip's events at their times from when it starts, which
# it marks by touching `mark` once System Events answers: reaching it takes long enough to throw
# the captions out.
def steps-script [events: list<record>, mark: path]: nothing -> string {
    let played = $events | where type != mark | reduce --fold {t: 0, lines: []} {|event, run|
        let wait = [($event.t - $run.t) 0] | math max | math round --precision 3
        let stroke = match $event.type {
            type => $"    keystroke \"($event.text | str replace --all '\' '\\' | str replace --all '"' '\"')\""
            key => (key-script (chord $event) | lines | get 1)
        }
        {t: $event.t, lines: ($run.lines | append [$"    delay ($wait)" $stroke])}
    }
    [
        'tell application "System Events" to get name of first process whose frontmost is true'
        $"do shell script \"touch \" & quoted form of \"($mark)\""
        'tell application "System Events"'
        ...$played.lines
        'end tell'
    ] | str join (char nl)
}

# A clip event's key as scripts/e2e/keys.nu names it, with its modifiers: `cmd+k`.
def chord [event: record]: nothing -> string {
    let names = {
        Enter: return
        Tab: tab
        Backspace: backspace
        Escape: escape
        Up: up
        Down: down
        Left: left
        Right: right
    }
    let key = $names | get --optional $event.name | default ($event.name | str lowercase)
    [ctrl alt shift cmd] | where {|modifier| $event | get --optional $modifier | default false } | append $key | str join +
}

def wait-for [what: string, ready: closure] {
    for _ in 1..100 {
        if (do $ready) { return }
        sleep 200ms
    }
    error make {msg: $"timed out waiting for ($what)"}
}
