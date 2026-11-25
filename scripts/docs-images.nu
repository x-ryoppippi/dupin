#!/usr/bin/env nu

use headless-config.nu [scenario-env panel-height headless]

const MANIFEST = path self docs-images/images.toml
const THEMES = [light dark]
# Pages draw at twice their size, so `width` in the Markdown is half the PNG's.
const SCALE = 2

# Render every image scripts/docs-images/images.toml lists to docs/images/<page>/<name>-<theme>.png,
# in both themes, and compress them losslessly.
#
# Each image comes from fixtures only, never from this Mac: an extension's own fixture through
# `dupin ext snap`, or a headless launcher scenario with a scratch home holding the fixture
# documents in ~/Documents, the stock apps in /System/Applications and the fixture clipboard
# history. Without `--only`, images the manifest no longer lists are deleted.
def main [
    --bin: path = "target/debug/dupin" # Any build of Dupin, or the app's Contents/MacOS/dupin
    --only: string # Render only the images whose `<page>/<name>` contains this
]: nothing -> table {
    let root = $env.FILE_PWD | path dirname
    cd $root
    let bin = $bin | path expand
    let manifest = open $MANIFEST
    let images = $manifest.images | where {|image| $only == null or (image-id $image | str contains $only) }
    if ($images | is-empty) {
        error make {msg: $"no image in ($MANIFEST | path basename) matches ($only)"}
    }
    let tmp = mktemp --directory | path expand
    let rendered = try {
        $images | each {|image| render $image $manifest $bin $tmp }
    } catch {|error|
        rm --recursive --force $tmp
        error make $error.raw
    }
    rm --recursive --force $tmp
    let files = $rendered | get png | flatten
    ^oxipng --quiet --opt 4 --strip safe ...$files
    if $only == null {
        let expected = $files | path expand
        glob docs/images/**/*.png | where {|png| $png not-in $expected } | each {|png| rm $png }
    }
    $rendered | update png {|row| $row.png | each { ls $in | get size.0 } | math sum } | rename image bytes
}

# Check that every image the manifest lists exists in both themes, that docs/images holds no
# other image, that the Markdown embeds every image in both themes and that every image the
# Markdown embeds exists. Pixels differ from Mac to Mac (fonts, app icons, the GPU), so this
# checks presence rather than rendering them again.
def "main check" []: nothing -> nothing {
    let root = $env.FILE_PWD | path dirname
    cd $root
    let problems = (
        problems (open $MANIFEST)
        (glob docs/images/**/*.png | each { path relative-to $root })
        (glob docs/**/*.md | where {|doc| not ($doc | str contains /design/) } | each {|doc|
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

export def image-id [image: record]: nothing -> string {
    $"($image.page)/($image.name)"
}

# The repository paths an image is saved at, one for each theme.
export def image-paths [image: record]: nothing -> list<string> {
    $THEMES | each {|theme| $"docs/images/(image-id $image)-($theme).png" }
}

# The images a Markdown document embeds, as repository paths: each `src` or `srcset` that is a
# relative path to a PNG, outside code blocks.
export def embedded [doc: string, text: string]: nothing -> list<string> {
    let folder = $doc | path dirname
    $text
    | str replace --all --regex '(?s)```.*?```' ''
    | parse --regex '(?:src|srcset)="(?P<url>[^"]+\.png)"'
    | get url
    | where {|url| not ($url =~ '^[a-z]+:') }
    | each {|url| resolve-dots ($folder | path join $url) }
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

# Everything wrong with the images: `files` are the images in docs/images, `docs` the Markdown
# documents with their text, and `exists` tells whether a repository path exists.
export def problems [
    manifest: record
    files: list<string>
    docs: list<record>
    exists: closure
]: nothing -> list<string> {
    let expected = $manifest.images | each {|image| image-paths $image } | flatten
    let embeds = $docs | each {|doc| embedded $doc.path $doc.text | each {|png| {doc: $doc.path, png: $png} } } | flatten
    let embedded = $embeds | get png? | default []
    let duplicates = $manifest.images | each {|image| image-id $image } | uniq --repeated
    [
        (
            $duplicates
            | each {|id| $"($MANIFEST | path basename) lists ($id) more than once" }
        )
        (
            $manifest.images
            | each {|image| source-problems $image $exists }
            | flatten
        )
        (
            $expected
            | where {|png| $png not-in $files }
            | each {|png| $"($png) is missing; run `just docs-images`" }
        )
        (
            $files
            | where {|png| $png not-in $expected }
            | each {|png| $"($png) is not in ($MANIFEST | path basename)" }
        )
        (
            $expected
            | where {|png| $png not-in $embedded }
            | each {|png| $"no document embeds ($png)" }
        )
        (
            $embeds
            | where {|embed| not (do $exists $embed.png) }
            | each {|embed| $"($embed.doc) embeds ($embed.png), which does not exist" }
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
    } else {
        return [$"($id) names neither an `extension` nor a `scenario`"]
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
    let pngs = if $image.extension? != null {
        render-extension $image $bin $tmp
    } else {
        render-scenario $image $manifest $bin $tmp
    }
    {image: $id, png: $pngs}
}

# An extension's fixture, through `dupin ext snap`, with its theme set to each of ours.
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
    image-paths $image | zip $THEMES | each {|pair|
        let themed = $tmp | path join $"(image-id $image | str replace --all / -)-($pair.1).json"
        open $fixture | upsert environment.theme $pair.1 | to json | save --force $themed
        headless $bin ext snap $image.extension $themed $pair.0 ...$step
        $pair.0
    }
}

# A headless launcher scenario, with its sidecar fixtures as scripts/snap.nu reads them. With a
# `window`, that window is drawn instead of the panel, at `width` and the height of its content.
#
# Each runs in a home of its own, with its configuration in ~/.config/dupin, so the paths Settings
# shows are the same on every Mac and in every run.
def render-scenario [
    image: record
    manifest: record
    bin: path
    tmp: path
]: nothing -> list<string> {
    let name = image-id $image | str replace --all / -
    let home = $tmp | path join $"($name)-home"
    make-documents ($home | path join Documents) $manifest.documents
    write-history $home $manifest.recent
    let scenario = scenario-env $image.scenario $tmp
    let config = $home | path join .config
    mkdir $config
    cp --recursive (
        $scenario.XDG_CONFIG_HOME?
        | default scripts/headless/config
        | path join dupin
    ) $config
    if $image.config? != null {
        cp $image.config ($config | path join dupin config.toml)
    }
    let environment = {
        XDG_DATA_HOME: ($tmp | path join $"($name)-data")
        DUPIN_HEADLESS_CLIPBOARD: (pwd | path join scripts/fixtures/clipboard.json)
    } | merge $scenario | merge {
        HOME: $home
        XDG_CONFIG_HOME: $config
        TMPDIR: ($tmp | path join tmp)
        LANG: en_GB.UTF-8
        LC_ALL: en_GB.UTF-8
        TZ: UTC
        DUPIN_HEADLESS_APPS: /System/Applications
    }
    mkdir $environment.TMPDIR
    let window = if $image.window? == null { [] } else { [--window $image.window] }
    let script = [
        --script
        $image.scenario
        --realtime
        --at
        mark:end
        ...$window
    ]
    let tree = $tmp | path join $"($name).json"
    with-env $environment { headless $bin ...$script --theme dark --tree $tree }
    let size = if $image.window? == null {
        $"680x(open $tree | panel-height)"
    } else {
        $"($image.width)x(open $tree | content-height)"
    }
    image-paths $image | zip $THEMES | each {|pair|
        with-env $environment {
            headless $bin ...$script --theme $pair.1 --snap $pair.0 --scale ($SCALE | into string) --size $size
        }
        $pair.0
    }
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
