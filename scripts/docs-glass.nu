#!/usr/bin/env nu

use docs-images.nu [
    MANIFEST
    GLASS
    CLIP_FPS
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
# The website's demo frame in points, whose shape every clip is cut to.
const FRAME = {width: 680, height: 578}

# The wallpapers the glass is recorded over, drawn here rather than shipped, each a landscape: a
# sky in bands of colour with a sun or moon, ridges of mountains fading into the haze, a treeline
# and a lake that reflects them. Their hard edges and contrasts are what the glass visibly blurs
# and bends. `sky` lists colours down to the horizon, `sun` sits at a fraction of the sky's
# width and height, each ridge's `base` and `amp` are fractions of the sky's height, `haze` is
# how much of the horizon's colour each ridge takes, and `stars` are drawn only in a dark sky.
const WALLPAPERS = {
    dusk: {
        sky: [
            "#1b1850"
            "#4b2f7e"
            "#a1447f"
            "#e8705f"
            "#fbab69"
            "#ffd592"
        ]
        sun: {
            x: 0.66
            y: 0.80
            r: 70
            colour: "#fff1cf"
            glow: "255,190,120"
        }
        ridges: [
            {
                base: 0.84
                amp: 0.22
                colour: "#c0668a"
                seed: 2.1
            }
            {
                base: 0.90
                amp: 0.24
                colour: "#7d4388"
                seed: 5.7
            }
            {
                base: 0.96
                amp: 0.20
                colour: "#3f2a6e"
                seed: 9.3
            }
        ]
        trees: "#150f33"
        water: "#1a1244"
        stars: 0
    }
    lagoon: {
        sky: ["#2a73c4" "#4fa0dc" "#8cc9ea" "#c5e6ee" "#f2f1de"]
        sun: {
            x: 0.30
            y: 0.60
            r: 60
            colour: "#fffbe6"
            glow: "255,250,220"
        }
        ridges: [
            {
                base: 0.84
                amp: 0.22
                colour: "#8fb4c8"
                seed: 1.3
            }
            {
                base: 0.91
                amp: 0.24
                colour: "#4f8a9a"
                seed: 4.4
            }
            {
                base: 0.97
                amp: 0.20
                colour: "#1f5a62"
                seed: 8.1
            }
        ]
        trees: "#0c2f35"
        water: "#14506a"
        stars: 0
    }
    night: {
        sky: ["#060b24" "#0d1c4a" "#1e3a7a" "#3f64a6" "#7a9ccc"]
        sun: {
            x: 0.72
            y: 0.34
            r: 52
            colour: "#f4f1e2"
            glow: "190,210,255"
        }
        ridges: [
            {
                base: 0.86
                amp: 0.20
                colour: "#355590"
                seed: 3.7
            }
            {
                base: 0.92
                amp: 0.22
                colour: "#1f3566"
                seed: 6.2
            }
            {
                base: 0.97
                amp: 0.18
                colour: "#0f1a3a"
                seed: 10.5
            }
        ]
        trees: "#060a1c"
        water: "#08122c"
        stars: 90
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
    --wallpaper: string # A preset (dusk, lagoon or night) or an image, for every clip
    --no-retina # Record on the display as it is, without a 2x virtual one, on CI
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
    let ci = ($env.CI? | default "") == "true"
    let retina = if $ci and not $no_retina { retina-display $stage }
    clear-transparency-settings $stage $ci
    print --stderr (^$stage displays)
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

# Liquid Glass shows what is behind it only when Reduce Transparency and Increase Contrast are
# off, or it is drawn as a solid panel. A CI runner is a throwaway VM, so on it they are turned
# off for the job; on a Mac in use they are only reported, since they are the user's.
def clear-transparency-settings [stage: path, ci: bool] {
    for key in [reduceTransparency increaseContrast] {
        let value = ^defaults read com.apple.universalaccess $key | complete
        print --stderr $"($key): ($value.stdout | str trim) ($value.stderr | str trim)"
        if $ci {
            ^defaults write com.apple.universalaccess $key -bool false
        }
    }
    sleep 1sec
    print --stderr (
        ^$stage displays
        | lines
        | where {|line| $line starts-with reduceTransparency }
        | str join (char nl)
    )
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

# The image a wallpaper names: a preset drawn at 2880×2000, the shape of the 1440×1000 virtual
# display at 2x, or a file as it is.
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
        draw-landscape $preset $tmp $name $out
    }
    $out
}

# A number in 0..1 that looks random and is the same every run.
def scatter [n: float]: nothing -> float {
    let x = ($n | math sin) * 43758.5453
    $x - ($x | math floor)
}

# A ridge's outline: 1 at a peak and 0 in a valley, sharp at the peaks, from three waves.
def ridge-height [x: int, seed: float]: nothing -> float {
    let t = $x / 2880.0 * 6.2832
    let a = 1.0 - (($t * 1.3 + $seed | math sin) | math abs)
    let b = 1.0 - (($t * 3.1 + $seed * 2.7 | math sin) | math abs)
    let c = 1.0 - (($t * 7.9 + $seed * 5.1 | math sin) | math abs)
    0.55 * $a + 0.3 * $b + 0.15 * $c
}

# The landscape a preset describes, to `out`: the sky and everything above the horizon first,
# then the lake, which is that scene turned upside down, rippled and darkened.
def draw-landscape [
    preset: record
    tmp: path
    name: string
    out: path
] {
    let width = 2880
    let height = 2000
    let horizon = 1240
    let bands = $preset.sky | window 2
    let band = ($horizon / ($bands | length)) | math ceil
    let sky = $bands | each {|pair|
        ["(" -size $"($width)x($band)" $"gradient:($pair.0)-($pair.1)" ")"]
    } | flatten
    let sun = $preset.sun
    let sun_x = $sun.x * $width | math round
    let sun_y = $sun.y * $horizon | math round
    let glow = $tmp | path join $"wallpaper-($name)-glow.png"
    ^magick -size 1600x1600 $"radial-gradient:rgba\(($sun.glow),0.85\)-rgba\(($sun.glow),0\)" $glow
    let stars = if $preset.stars > 0 {
        1..$preset.stars | each {|i|
            let x = (scatter ($i * 12.9898)) * $width | math round
            let y = (scatter ($i * 78.233)) * $horizon * 0.6 | math round
            let r = 1 + (scatter ($i * 3.7)) * 3 | math round
            [-fill "rgba(255,255,255,0.8)" -draw $"circle ($x),($y) ($x + $r),($y)"]
        } | flatten
    } else {
        []
    }
    let ridges = $preset.ridges | each {|ridge|
        let points = 0..($width // 16) | each {|i|
            let x = $i * 16
            let y = $horizon * ($ridge.base - $ridge.amp * (ridge-height $x $ridge.seed)) | math round
            $"($x),($y)"
        }
        let outline = $points | append [$"($width),($horizon)" $"0,($horizon)"] | str join " "
        [-fill $ridge.colour -draw $"polygon ($outline)"]
    } | flatten
    let trees = 0..($width // 11) | each {|i|
        let x = $i * 11 + ((scatter ($i * 5.31)) * 8 | math round)
        let tall = 34 + (scatter ($i * 9.17)) * 70 | math round
        [-fill $preset.trees -draw $"polygon ($x - 9),($horizon + 2) ($x),($horizon - $tall) ($x + 9),($horizon + 2)"]
    } | flatten
    let scene = $tmp | path join $"wallpaper-($name)-scene.png"
    (
        ^magick ...$sky -append -resize $"($width)x($horizon)!"
        ...$stars
        $glow -geometry $"+($sun_x - 800)+($sun_y - 800)" -composite
        -fill $sun.colour -draw $"circle ($sun_x),($sun_y) ($sun_x + $sun.r),($sun_y)"
        ...$ridges
        -fill $preset.trees -draw $"rectangle 0,($horizon - 10) ($width),($horizon)"
        ...$trees
        $scene
    )
    let lake = $height - $horizon
    let reflection = $tmp | path join $"wallpaper-($name)-reflection.png"
    (
        ^magick $scene -flip -crop $"($width)x($lake)+0+0" +repage
        -motion-blur 0x28+0 -blur 0x3 -modulate 92,90
        -fill $preset.water -colorize 18%
        "(" -size $"($width)x($lake)" gradient:white-#4a4a4a ")" -compose multiply -composite
        $reflection
    )
    (
        ^magick $scene $reflection -append
        -attenuate 0.06 +noise Gaussian -depth 8
        $out
    )
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
                -c:v libx264 -preset veryfast -crf 8 -pix_fmt yuv444p -stats_period 0.1
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
        print --stderr (^$stage windows)
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
    let x = [
        ($panel.x - $margin)
        0
    ] | math max
    let y = [
        ($panel.y - $margin)
        0
    ] | math max
    # Every clip is cut to the same rectangle: the aspect ratio of the page's demo frame (`FRAME`)
    # with the wallpaper around the panel, so the frame is filled whichever tab shows and
    # switching never moves it. It must cover the tallest panel with its captions, and end above
    # the Dock, which is not wallpaper. H.264 takes only even sides.
    let width = ($FRAME.width * $scale + 2 * $margin) // 2 * 2
    let height = ($width * $FRAME.height / $FRAME.width | math ceil) // 2 * 2 + 2
    if $height > $panel.bottom {
        error make {
            msg: $"($id): a ($width)x($height) cut doesn't fit above the Dock, at ($panel.bottom)"
        }
    }
    # Sliding it up when the panel is too low for the cut to end above the Dock, by the same
    # amount for every clip as the panel opens in the same place.
    let y = [
        $y
        ($panel.bottom - $height)
    ] | math min
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
