#!/usr/bin/env nu

use headless-config.nu [scenario-env panel-height headless]

const MANIFEST = path self docs-images/images.toml
const THEMES = [light dark]
# Pages draw at twice their size, so `width` in the Markdown is half the PNG's.
const SCALE = 2
# Clips draw at 1.25 times their size and 10 frames a second: a clip's bytes go to the frames
# that differ, so these keep a busy one, such as scrolling emoji, near 250 KB, and text stays
# readable on a Retina screen.
const CLIP_SCALE = 1.25
const CLIP_FPS = 10
# Clips render this many points tall, more than the panel ever is, and are cut to it.
const CLIP_HEIGHT = 720
# Room below the panel for the captions, so they never cover a row.
const CAPTION_SPACE = 60
# How a clip's steps are paced, in seconds.
const CLIP_START = 0.6
const CLIP_TYPING = 0.12
const CLIP_PAUSE = 0.8
const CLIP_HOLD = 1.6
# Keys a replay presses closer together than this share one caption.
const CAPTION_JOIN = 0.4

# The keys a clip's steps name, as Rune names them and as a caption shows them.
const KEYS = {
    space: {name: " ", caption: "␣ Space"}
    tab: {name: Tab, caption: "⇥ Tab"}
    enter: {name: Enter, caption: "↩ Return"}
    escape: {name: Escape, caption: "⎋ Esc"}
    backspace: {name: Backspace, caption: "⌫ Delete"}
    up: {name: Up, caption: "↑ Up"}
    down: {name: Down, caption: "↓ Down"}
    left: {name: Left, caption: "← Left"}
    right: {name: Right, caption: "→ Right"}
}

# Render every image and clip scripts/docs-images/images.toml lists, in both themes: images to
# docs/images/<page>/<name>-<theme>.png, compressed losslessly, and clips to
# docs/images/<page>/<name>-<theme>.webp, animated, with the keys they press drawn below the panel.
#
# Each comes from fixtures only, never from this Mac: an extension's own fixture through
# `dupin ext render`, or a headless launcher scenario with a scratch home holding the fixture
# documents in ~/Documents, the stock apps in /System/Applications and the fixture clipboard
# history. Without `--only`, files of the kinds rendered that the manifest no longer lists are
# deleted.
def main [
    --bin: path = "target/debug/dupin" # Any build of Dupin, or the app's Contents/MacOS/dupin
    --only: string # Render only the images and clips whose `<page>/<name>` contains this
    --kind: string = "all" # `images`, `clips` or `all`
]: nothing -> table {
    let root = $env.FILE_PWD | path dirname
    cd $root
    let bin = $bin | path expand
    let manifest = open $MANIFEST
    let kinds = match $kind {
        all => [images clips]
        "images" | "clips" => [$kind]
        _ => (error make {msg: $"--kind is `images`, `clips` or `all`, not `($kind)`"})
    }
    let entries = manifest-entries $manifest $kinds
    let entries = $entries | where {|entry| $only == null or (image-id $entry | str contains $only) }
    if ($entries | is-empty) {
        error make {msg: $"nothing in ($MANIFEST | path basename) matches ($only)"}
    }
    let tmp = mktemp --directory | path expand
    let rendered = try {
        $entries | each {|entry| render $entry $manifest $bin $tmp }
    } catch {|error|
        rm --recursive --force $tmp
        error make $error.raw
    }
    rm --recursive --force $tmp
    let files = $rendered | get files | flatten
    let pngs = $files | where {|file| $file | str ends-with .png }
    if ($pngs | is-not-empty) {
        ^oxipng --quiet --opt 4 --strip safe ...$pngs
    }
    if $only == null {
        let expected = $files | path expand
        $kinds
        | each {|kind| kind-extensions $kind | each {|extension| glob $"docs/images/**/*.($extension)" } }
        | flatten
        | flatten
        | where {|file| $file not-in $expected }
        | each {|file| rm $file }
    }
    $rendered | update files {|row| $row.files | each { ls $in | get size.0 } | math sum } | rename image bytes
}

# Check that every image and clip the manifest lists exists in each of its themes, that
# docs/images holds nothing else, that the Markdown or the website embeds each and that
# everything they embed exists. Pixels differ from Mac to Mac (fonts, app icons, the GPU), so this
# checks presence rather than rendering them again.
def "main check" []: nothing -> nothing {
    let root = $env.FILE_PWD | path dirname
    cd $root
    let problems = (
        problems (open $MANIFEST)
        (glob docs/images/**/*.{png,webp,mp4} | each { path relative-to $root })
        (glob docs/**/*.md | where {|doc| not ($doc | str contains /design/) } | append (glob website/src/**/*.{svelte,ts} | where {|file| not ($file | str ends-with .test.ts) }) | each {|doc|
            {path: ($doc | path relative-to $root), text: (open --raw $doc)}
        })
        {|path| $path | path exists }
    )
    if ($problems | is-not-empty) {
        error make {
            msg: ($problems | str join "\n")
        }
    }
}

# The extensions the files of `kind` have: clips are animated WebP, or MP4 with a WebP poster.
def kind-extensions [kind: string]: nothing -> list<string> {
    if $kind == images { [png] } else { [webp mp4] }
}

export def image-id [image: record]: nothing -> string {
    $"($image.page)/($image.name)"
}

# The manifest's entries of `kinds`, each clip marked as one.
def manifest-entries [manifest: record, kinds: list<string>]: nothing -> list<record> {
    $kinds | each {|kind|
        $manifest | get --optional $kind | default [] | each {|entry|
            if $kind == clips { $entry | upsert clip true } else { $entry }
        }
    } | flatten
}

# The themes an image or clip is rendered in: both, unless it names `themes`.
export def image-themes [image: record]: nothing -> list<string> {
    $image.themes? | default $THEMES
}

# The repository path an image or clip is saved at in `theme`.
export def theme-path [image: record, theme: string]: nothing -> string {
    let clip = ($image.clip? | default false) or $image.steps? != null
    let extension = if not $clip { "png" } else if ($image.video? | default false) { "mp4" } else { "webp" }
    $"docs/images/(image-id $image)-($theme).($extension)"
}

# The still a video clip shows before it plays, and in place of it under reduced motion: its
# last frame.
export def poster-path [image: record, theme: string]: nothing -> string {
    $"docs/images/(image-id $image)-($theme)-poster.webp"
}

# The repository paths an image or clip is saved at: one for each theme, and a video clip's
# poster beside each.
export def image-paths [image: record]: nothing -> list<string> {
    image-themes $image | each {|theme|
        if ($image.video? | default false) {
            [(theme-path $image $theme) (poster-path $image $theme)]
        } else {
            [(theme-path $image $theme)]
        }
    } | flatten
}

# The images and clips a Markdown document embeds, as repository paths: each `src` or `srcset`
# that is a relative path to a PNG or WebP, outside code blocks.
export def embedded [doc: string, text: string]: nothing -> list<string> {
    let folder = $doc | path dirname
    let text = $text | str replace --all --regex '(?s)```.*?```' ''
    let relative = $text
    | parse --regex '(?:src|srcset|poster)="(?P<url>[^"/][^"]*\.(?:png|webp|mp4))"'
    | get url
    | where {|url| not ($url =~ '^[a-z]+:') }
    | each {|url| resolve-dots ($folder | path join $url) }
    # The website's pages name the images at the paths the site serves them from.
    let served = $text
    | parse --regex `["'](?P<url>/docs/images/[^"']+\.(?:png|webp|mp4))["']`
    | get url
    | each { str substring 1.. }
    $relative | append $served
}

def resolve-dots [path: string]: nothing -> string {
    $path | split row / | reduce --fold [] {|part, kept|
        match $part {
            "." => $kept
            ".." => ($kept | drop)
            _ => ($kept | append $part)
        }
    } | str join /
}

# Everything wrong with the images and clips: `files` are those in docs/images, `docs` the
# Markdown documents with their text, and `exists` tells whether a repository path exists.
export def problems [
    manifest: record
    files: list<string>
    docs: list<record>
    exists: closure
]: nothing -> list<string> {
    let entries = manifest-entries $manifest [images clips]
    let expected = $entries | each {|image| image-paths $image } | flatten
    let embeds = $docs | each {|doc| embedded $doc.path $doc.text | each {|file| {doc: $doc.path, file: $file} } } | flatten
    let embedded = $embeds | get file? | default []
    let duplicates = $entries | each {|image| image-id $image } | uniq --repeated
    [
        (
            $duplicates
            | each {|id| $"($MANIFEST | path basename) lists ($id) more than once" }
        )
        (
            $entries
            | each {|image| source-problems $image $exists }
            | flatten
        )
        (
            $expected
            | where {|file| $file not-in $files }
            | each {|file| $"($file) is missing; run `just docs-images`" }
        )
        (
            $files
            | where {|file| $file not-in $expected }
            | each {|file| $"($file) is not in ($MANIFEST | path basename)" }
        )
        (
            $expected
            | where {|file| $file not-in $embedded }
            | each {|file| $"no document embeds ($file)" }
        )
        (
            $embeds
            | where {|embed| not (do $exists $embed.file) }
            | each {|embed| $"($embed.doc) embeds ($embed.file), which does not exist" }
        )
    ] | flatten
}

def source-problems [image: record, exists: closure]: nothing -> list<string> {
    let id = image-id $image
    let sources = if $image.extension? != null {
        [
            $image.extension
            ($image.extension | path join fixtures $image.fixture)
        ]
    } else if $image.scenario? != null {
        [$image.scenario] | append ($image.config? | default [])
    } else if $image.steps? != null {
        if ($image.steps | is-empty) {
            return [$"($id) has no steps"]
        }
        [] | append ($image.fixtures? | default []) | append ($image.config? | default [])
    } else {
        return [$"($id) names neither an `extension`, a `scenario` nor `steps`"]
    }
    $sources | where {|source| not (do $exists $source) } | each {|source| $"($id) renders from ($source), which does not exist" }
}

def render [
    image: record
    manifest: record
    bin: path
    tmp: path
]: nothing -> record {
    let id = image-id $image
    print --stderr $"rendering ($id)"
    let folder = $"docs/images/($image.page)"
    mkdir $folder
    let files = if $image.extension? != null and ($image.clip? | default false) {
        render-extension-clip $image $bin $tmp
    } else if $image.extension? != null {
        render-extension $image $bin $tmp
    } else if $image.steps? != null {
        render-clip $image $manifest $bin $tmp
    } else {
        render-scenario $image $manifest $bin $tmp
    }
    {image: $id, files: $files}
}

# An extension's fixture, through `dupin ext render`, with its theme set to each of ours.
def render-extension [image: record, bin: path, tmp: path]: nothing -> list<string> {
    let fixture = $image.extension | path join fixtures $image.fixture
    let step = if $image.step? == null {
        []
    } else {
        [
            --step
            ($image.step | into string)
        ]
    }
    image-paths $image | zip (image-themes $image) | each {|pair|
        let themed = $tmp | path join $"(image-id $image | str replace --all / -)-($pair.1).json"
        open $fixture | upsert environment.theme $pair.1 | to json | save --force $themed
        headless $bin ext render $image.extension $themed $pair.0 ...$step
        $pair.0
    }
}

# A headless launcher scenario, with its sidecar fixtures as scripts/render.nu reads them. With a
# `window`, that window is drawn instead of the panel, at `width` and the height of its content.
def render-scenario [
    image: record
    manifest: record
    bin: path
    tmp: path
]: nothing -> list<string> {
    let environment = scratch-environment $image.scenario $image $manifest $tmp
    let window = if $image.window? == null { [] } else { [--window $image.window] }
    let script = [
        --script
        $image.scenario
        --realtime
        --at
        mark:end
        ...$window
    ]
    let tree = $tmp | path join $"(image-id $image | str replace --all / -).json"
    with-env $environment { headless $bin ...$script --theme dark --tree $tree }
    let size = if $image.window? == null {
        $"680x(open $tree | panel-height)"
    } else {
        $"($image.width)x(open $tree | content-height)"
    }
    image-paths $image | zip (image-themes $image) | each {|pair|
        with-env $environment {
            headless $bin ...$script --theme $pair.1 --snap $pair.0 --scale ($SCALE | into string) --size $size
        }
        $pair.0
    }
}

# A clip: its `steps` played into the panel in real time, with the sidecar fixtures of the
# scenario `fixtures` names, recorded as an animated WebP with a caption below the panel naming
# each key pressed or the text typed. The panel is as tall as it gets after any step.
def render-clip [
    clip: record
    manifest: record
    bin: path
    tmp: path
]: nothing -> list<string> {
    let name = image-id $clip | str replace --all / -
    let timeline = clip-timeline $clip.steps
    let script = $tmp | path join $"($name)-script.json"
    {events: $timeline.events} | to json | save $script
    let environment = (scratch-environment
        ($clip.fixtures? | default $script)
        $clip
        $manifest
        $tmp
    )
    let run = [--script $script --realtime]
    let scale = $clip.scale? | default $CLIP_SCALE
    let captions = if ($clip.captions? | default true) { $timeline.captions } else { null }
    image-themes $clip | each {|theme|
        let frames = $tmp | path join $"($name)-($theme)-frames"
        with-env $environment {
            (
                headless $bin ...$run --theme $theme --size $"680x($CLIP_HEIGHT)"
                --scale ($scale | into string) --fps ($CLIP_FPS | into string) --out $frames
            )
        }
        let pngs = glob ($frames | path join frame_*.png) | sort
        let out = theme-path $clip $theme
        let video = if ($clip.video? | default false) {
            {poster: (poster-path $clip $theme), background: ($clip.background? | default "#000000")}
        }
        finish-clip $pngs $captions $out ($tmp | path join $"($name)-captions") $scale $video
        [$out] | append ($video | get --optional poster | default [])
    } | flatten
}

# Cuts `pngs`, a clip's frames at `scale`, to the lowest edge the panel reaches in any
# frame, since it grows and shrinks as results change, with room for the captions below unless
# `captions` is null; draws the captions; and encodes them at `out`: as an animated WebP, or with
# `video`, over its `background` as an MP4 with its last frame as a WebP `poster`.
def finish-clip [
    pngs: list<string>
    captions
    out: path
    folder: path
    scale: number
    video?: record
]: nothing -> nothing {
    let bottom = ^magick ...$pngs -background none -layers merge -format "%@" info: | parse "{w}x{h}+{x}+{y}" | first | ($in.h | into int) + ($in.y | into int)
    let width = ^magick identify -format "%w" ($pngs | first) | into int
    let space = if $captions == null {
        0
    } else {
        $CAPTION_SPACE * $scale | math round
    }
    # H.264 takes only even sides.
    let height = ($bottom + $space + 1) // 2 * 2
    ^magick mogrify -gravity north -background none -crop $"($width)x($bottom)+0+0" +repage -extent $"($width)x($height)" ...$pngs
    if $captions != null {
        caption $pngs $captions $folder $scale
    }
    if $video != null {
        encode-video $pngs $out $video
        return
    }
    # Lossy frames: lossless ones are several times larger, and the panel's soft glass hides
    # the difference. Frames that repeat the last cost nothing.
    let result = ^img2webp -loop 0 -lossy -q 75 -m 6 -d (1000 // $CLIP_FPS | into string) ...$pngs -o $out | complete
    if $result.exit_code != 0 {
        error make {msg: $"img2webp failed for ($out):\n($result.stderr)"}
    }
}

# Encodes `pngs` over `video.background` as an H.264 MP4 at `out`, which every browser plays and
# which takes a fraction of an animated WebP's bytes when a backdrop shader changes every pixel of
# every frame, and saves the last frame as `video.poster`.
def encode-video [pngs: list<string>, out: path, video: record]: nothing -> nothing {
    ^magick mogrify -background $video.background -flatten ...$pngs
    let frames = $pngs | first | path dirname | path join "frame_*.png"
    let result = (
        ^ffmpeg -loglevel error -y -framerate ($CLIP_FPS | into string) -pattern_type glob -i $frames
        -c:v libx264 -crf 26 -preset veryslow -tune animation -pix_fmt yuv420p -movflags +faststart -an $out
        | complete
    )
    if $result.exit_code != 0 {
        error make {msg: $"ffmpeg failed for ($out):\n($result.stderr)"}
    }
    ^magick ($pngs | last) -quality 85 $video.poster
}

# An extension's fixture played through `dupin ext render --record`, up to `step` or its last
# step, as a clip: the replay's own keys, captioned from the script it records beside the
# frames.
def render-extension-clip [clip: record, bin: path, tmp: path]: nothing -> list<string> {
    let name = image-id $clip | str replace --all / -
    let fixture = $clip.extension | path join fixtures $clip.fixture
    let step = if $clip.step? == null {
        []
    } else {
        [
            --step
            ($clip.step | into string)
        ]
    }
    let scale = open $fixture | get --optional environment.window.scale | default 2
    image-paths $clip | zip (image-themes $clip) | each {|pair|
        let themed = $tmp | path join $"($name)-($pair.1).json"
        open $fixture | upsert environment.theme $pair.1 | to json | save --force $themed
        let frames = $tmp | path join $"($name)-($pair.1)-frames"
        let still = $tmp | path join $"($name)-($pair.1).png"
        headless $bin ext render $clip.extension $themed $still ...$step --record $frames
        let pngs = glob ($frames | path join frame_*.png) | sort
        let points = (^magick identify -format "%w" ($pngs | first) | into int) / $scale
        ^magick mogrify -resize $"($points * $CLIP_SCALE | math round)x" ...$pngs
        let captions = script-captions (open ($frames | path join script.json)).events
        finish-clip $pngs $captions $pair.0 ($tmp | path join $"($name)-captions") $CLIP_SCALE
        $pair.0
    }
}

# Captions for a recorded Rune script: typed text as it grows, and keys pressed within
# `CAPTION_JOIN` seconds of each other as one caption, since a replay presses them faster than a
# caption could be read.
export def script-captions [events: list<record>]: nothing -> list<record> {
    let names = $KEYS | transpose key spec | each {|key| {name: $key.spec.name, key: $key.key} }
    $events | each {|event|
        match $event.type {
            "type" if ($event.text | str trim | is-empty) => {
                [{t: $event.t, text: (key-caption {key: space}), key: true}]
            }
            "type" => {
                let chars = $event.text | split chars
                $chars | enumerate | each {|char|
                    {
                        t: ($event.t + $char.index * ($event.every? | default 0))
                        text: $"type: ($chars | first ($char.index + 1) | str join | str trim)"
                        key: false
                    }
                }
            }
            "key" => {
                let key = $names | where name == $event.name | get --optional 0.key | default ($event.name | str lowercase)
                [{t: $event.t, text: (key-caption ($event | reject t type name | insert key $key)), key: true}]
            }
            _ => []
        }
    } | flatten | where text != "type: " | insert latest {|caption| $caption.t } | reduce --fold [] {|caption, joined|
        let last = if ($joined | is-empty) { null } else { $joined | last }
        if ($last != null and $caption.key and $last.key and $caption.t - $last.latest < $CAPTION_JOIN) {
            $joined | drop | append ($last | update text $"($last.text)  ($caption.text)" | update latest $caption.t)
        } else {
            $joined | append $caption
        }
    } | reject key latest
}

# Draws each caption on the frames it is shown in, as a pill centred at the bottom: from its
# time until the next caption's, or `CLIP_HOLD` seconds, whichever is sooner. One style for both
# themes, in Inter, which `DUPIN_CAPTION_FONT` names (the dev shell sets it).
def caption [
    frames: list<string>
    captions: list<record>
    folder: path
    scale: number
]: nothing -> nothing {
    mkdir $folder
    let font = $env.DUPIN_CAPTION_FONT
    let px = {|points|
        $points * $scale | math round
    }
    let spans = $captions | enumerate | each {|entry|
        let next = $captions | get --optional ($entry.index + 1) | get --optional t | default 1e9
        {text: $entry.item.text, from: $entry.item.t, until: ([$next ($entry.item.t + $CLIP_HOLD)] | math min)}
    }
    let shown = $frames | enumerate | each {|frame|
        let t = $frame.index / $CLIP_FPS
        {frame: $frame.item, text: ($spans | where {|span| $t >= $span.from and $t < $span.until } | get --optional 0.text)}
    } | where text != null
    $shown | chunk-by {|frame| $frame.text } | each {|run|
        let text = $run.0.text
        let pill = $folder | path join $"($text | hash md5).png"
        if not ($pill | path exists) {
            let width = ^magick -font $font -pointsize (do $px 15) $"label:($text)" -format %w info: | into int
            let w = $width + (do $px 28)
            let h = do $px 32
            (
                ^magick -size $"($w)x($h)" xc:none -fill "#1d1d1fe6" -stroke "#ffffff2e" -strokewidth (do $px 1)
                -draw $"roundrectangle 0,0 ($w - 1),($h - 1) ($h / 2),($h / 2)"
                -fill white -stroke none -font $font -pointsize (do $px 15) -gravity center
                -annotate +0+0 $text $pill
            )
        }
        ^magick mogrify -gravity south -draw $"image over 0,(do $px 16) 0,0 '($pill)'" ...($run | get frame)
    } | ignore
}

# A clip's steps as a Rune script and the captions that name them: `{type: "cal"}` types one
# character at a time, or all at once with `burst`, `{key: "tab"}` (with `cmd`, `shift`, `alt` or `ctrl`) presses a key, and
# `{wait: 1.5}` waits. Each step but a wait is followed by a pause, and the last by a hold before
# the `end` mark.
export def clip-timeline [steps: list<record>]: nothing -> record {
    let start = {
        t: $CLIP_START
        events: []
        captions: []
    }
    let played = $steps | reduce --fold $start {|step, run| clip-step $step $run }
    {
        events: (
            $played.events
            | append {t: ($played.t + $CLIP_HOLD), type: mark, name: end}
        )
        captions: $played.captions
    }
}

def clip-step [step: record, run: record]: nothing -> record {
    if $step.wait? != null {
        $run | update t ($run.t + $step.wait)
    } else if $step.type? != null and ($step.burst? | default false) {
        {
            t: ($run.t + $CLIP_PAUSE)
            events: ($run.events | append {t: $run.t, type: type, text: $step.type})
            captions: ($run.captions | append {t: $run.t, text: $"type: ($step.type)"})
        }
    } else if $step.type? != null {
        let chars = $step.type | split chars
        let typed = $chars | enumerate | each {|char|
                let t = $run.t + $char.index * $CLIP_TYPING
                {
                    event: {t: $t, type: type, text: $char.item}
                    caption: {t: $t, text: $"type: ($chars | first ($char.index + 1) | str join)"}
                }
            }
        {
            t: ($run.t + ($chars | length) * $CLIP_TYPING + $CLIP_PAUSE)
            events: ($run.events | append ($typed | get event))
            captions: ($run.captions | append ($typed | get caption))
        }
    } else if $step.key? != null {
        let modifiers = $step | reject key
        let event = if $step.key == space and ($modifiers | is-empty) {
            {t: $run.t, type: type, text: " "}
        } else {
            {
                t: $run.t
                type: key
                name: (key-name $step.key)
            } | merge $modifiers
        }
        {
            t: ($run.t + $CLIP_PAUSE)
            events: ($run.events | append $event)
            captions: ($run.captions | append {t: $run.t, text: (key-caption $step)})
        }
    } else {
        error make {msg: $"a clip step is `type`, `key` or `wait`, not ($step | to nuon)"}
    }
}

def key-name [key: string]: nothing -> string {
    $KEYS | get --optional $key | get --optional name | default $key
}

# A key as the caption names it: modifiers as macOS draws them, then the key, as in `⌘K`.
export def key-caption [step: record]: nothing -> string {
    let modifiers = [[flag, glyph]; [ctrl, ⌃] [alt, ⌥] [shift, ⇧] [cmd, ⌘]]
    | where {|modifier| $step | get --optional $modifier.flag | default false }
    | get glyph
    | str join
    let key = $KEYS | get --optional $step.key
    if $key == null {
        $"($modifiers)($step.key | str uppercase)"
    } else if ($modifiers | is-empty) {
        $key.caption
    } else {
        $"($modifiers)($key.caption | split row ' ' | first)"
    }
}

# A scratch home and configuration for a headless run, with the sidecar fixtures beside
# `scenario`, the manifest's documents and history, and an entry's own `config`.
#
# Each runs in a home of its own, with its configuration in ~/.config/dupin, so the paths Settings
# shows are the same on every Mac and in every run.
def scratch-environment [
    scenario: path
    image: record
    manifest: record
    tmp: path
]: nothing -> record {
    let name = image-id $image | str replace --all / -
    let home = $tmp | path join $"($name)-home"
    make-documents ($home | path join Documents) $manifest.documents
    write-history $home $manifest.recent
    let fixtures = scenario-env $scenario $tmp
    let config = $home | path join .config
    mkdir $config
    cp --recursive (
        $fixtures.XDG_CONFIG_HOME?
        | default scripts/headless/config
        | path join dupin
    ) $config
    if $image.config? != null {
        cp $image.config ($config | path join dupin config.toml)
    }
    # `backdrop` picks an example shader as Settings does, from the `shaders` folder.
    cp --recursive examples/shaders ($config | path join dupin shaders)
    if $image.backdrop? != null {
        $"\nbackdrop = ($image.backdrop | to json)\n" | save --append ($config | path join dupin config.toml)
    }
    let environment = {
        XDG_DATA_HOME: ($tmp | path join $"($name)-data")
        DUPIN_HEADLESS_CLIPBOARD: (pwd | path join scripts/fixtures/clipboard.json)
    } | merge $fixtures | merge {
        HOME: $home
        XDG_CONFIG_HOME: $config
        TMPDIR: ($tmp | path join tmp)
        LANG: en_GB.UTF-8
        LC_ALL: en_GB.UTF-8
        TZ: UTC
        DUPIN_HEADLESS_APPS: /System/Applications
    }
    mkdir $environment.TMPDIR
    $environment
}

# How tall a window must be to show all of its content, in points: the lowest edge of any laid
# out element, with the window's own margin below it.
def content-height []: record -> int {
    let bottom = $in.elements | where rect? != null | each {|element| $element.rect.1 + $element.rect.3 } | math max
    $bottom + 16 | math ceil
}

# A usage history in which the home opened `recent`, the first most often, each an hour before
# the next, so Recent items lists them in that order.
def write-history [home: path, recent: list<string>]: nothing -> nothing {
    let now = date now | into int | $in // 1_000_000_000
    let items = $recent | enumerate | each {|entry|
        let count = ($recent | length) - $entry.index
        let visits = 1..$count | each {|visit| $now - $entry.index * 3600 - $visit }
        {($entry.item | str replace --regex '^~' $home): {count: $count, visits: ($visits | reverse), queries: {}}}
    } | reduce --fold {} {|item, all| $all | merge $item }
    let support = $home | path join Library "Application Support" Dupin
    mkdir $support
    {items: $items} | to json | save ($support | path join history.json)
}

# The manifest's fixture documents, as files that name themselves; folders end with a slash.
def make-documents [folder: path, documents: list<string>]: nothing -> nothing {
    mkdir $folder
    for document in $documents {
        let file = $folder | path join $document
        if ($document | str ends-with /) {
            mkdir $file
        } else {
            mkdir ($file | path dirname)
            $"Dupin documentation fixture: ($document)\n" | save $file
        }
    }
}
