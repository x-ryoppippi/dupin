use std/assert
use ../docs-images.nu [
    clip-timeline
    composite-problems
    composite-sources
    glass-problems
    embedded
    image-paths
    key-caption
    problems
    script-captions
]

const SOURCES = [
    extensions/calendar
    extensions/calendar/fixtures/upcoming.json
    scripts/docs-images/search.json
]

def picture [doc_folder: string, id: string]: nothing -> string {
    let media = '(prefers-color-scheme: dark)'
    $'<picture><source media="($media)" srcset="($doc_folder)images/($id)-dark.png"><img alt="x" src="($doc_folder)images/($id)-light.png"></picture>'
}

def main [] {
    let calendar = {
        page: calendar
        name: upcoming
        extension: extensions/calendar
        fixture: upcoming.json
    }
    let search = {page: usage, name: search, scenario: scripts/docs-images/search.json}
    let manifest = {
        images: [$calendar $search]
    }
    let files = [$calendar $search] | each {|image| image-paths $image } | flatten
    let docs = [
        {
            path: docs/usage.md
            text: (picture "" usage/search)
        }
        {
            path: docs/features/calendar.md
            text: (picture "../" calendar/upcoming)
        }
    ]
    let exists = {|path| $path in ($files | append $SOURCES) }

    assert equal (image-paths $calendar) [
        docs/images/calendar/upcoming-light.png
        docs/images/calendar/upcoming-dark.png
    ]
    assert equal (embedded docs/features/calendar.md (picture "../" calendar/upcoming)) [
        docs/images/calendar/upcoming-dark.png
        docs/images/calendar/upcoming-light.png
    ]
    assert equal (
        embedded docs/usage.md '<img src="https://example.com/a.png"> ![a](images/x.png)'
    ) []
    assert equal (
        embedded docs/usage.md "```html\n<img src=\"images/x.png\" />\n```\n<img src=\"images/y.png\" />"
    ) [docs/images/y.png]
    assert equal (problems $manifest $files $docs $exists) []

    let missing = problems $manifest ($files | skip 1) $docs $exists
    assert (
        $missing
        | any { str contains "docs/images/calendar/upcoming-light.png is missing" }
    )

    let stray = problems $manifest ($files | append docs/images/old/gone-dark.png) $docs $exists
    assert ($stray | any { str contains "docs/images/old/gone-dark.png is not in" })

    let unembedded = problems $manifest $files ($docs | first 1) $exists
    assert equal ($unembedded | length) 2
    assert (
        $unembedded
        | all { str starts-with "no document embeds docs/images/calendar/upcoming-" }
    )

    let dangling = problems $manifest $files (
        $docs
        | append {path: docs/features/emoji.md, text: (picture "../" emoji/gone)}
    ) $exists
    assert (
        $dangling
        | any { str contains "docs/features/emoji.md embeds docs/images/emoji/gone-light.png, which does not exist" }
    )

    let unsourced = problems {images: [($search | update scenario scripts/docs-images/gone.json)]} $files ($docs | first 1) $exists
    assert (
        $unsourced
        | any { str contains "renders from scripts/docs-images/gone.json, which does not exist" }
    )

    let twice = problems {images: [$search $search]} ($files | skip 2) ($docs | first 1) $exists
    assert ($twice | any { str contains "lists usage/search more than once" })

    let clip = {
        page: tutorial
        name: apps
        steps: [
            {type: "ab"}
            {key: k, cmd: true}
        ]
    }
    let clip_files = image-paths $clip
    assert equal $clip_files [
        docs/images/tutorial/apps-light.webp
        docs/images/tutorial/apps-dark.webp
    ]
    let clip_doc = {
        path: docs/tutorial.md
        text: (picture "" tutorial/apps | str replace --all .png .webp)
    }
    let clip_exists = {|path| $path in ($files | append $clip_files | append $SOURCES) }
    assert equal (
        problems {images: [$search], clips: [$clip]} ($files | skip 2 | append $clip_files) [($docs | first) $clip_doc] $clip_exists
    ) []
    let unrendered = problems {images: [$search], clips: [$clip]} ($files | skip 2) [
        ($docs | first)
        $clip_doc
    ] $clip_exists
    assert (
        $unrendered
        | any { str contains "docs/images/tutorial/apps-dark.webp is missing" }
    )
    let stepless = problems {images: [], clips: [($clip | update steps [])]} $clip_files [$clip_doc] $clip_exists
    assert ($stepless | any { str contains "tutorial/apps has no steps" })

    let timeline = clip-timeline [
        {type: "ab"}
        {wait: 1.0}
        {key: k, cmd: true}
        {key: space}
        {type: "cd", burst: true}
    ]
    assert equal ($timeline.events | where type != mark | each { reject t }) [
        {type: type, text: a}
        {type: type, text: b}
        {type: key, name: k, cmd: true}
        {type: type, text: " "}
        {type: type, text: cd}
    ]
    assert equal ($timeline.captions | get text) ["type: a" "type: ab" "⌘K" "␣ Space" "type: cd"]
    let times = $timeline.events | get t
    assert equal $times ($times | sort)
    assert equal ($timeline.events | last | get name) end

    assert equal (key-caption {key: tab}) "⇥ Tab"
    assert equal (key-caption {key: tab, shift: true}) "⇧⇥"
    assert equal (key-caption {key: enter, cmd: true}) "⌘↩"

    let recorded = [
        {
            t: 1.0
            type: type
            text: "ab"
            every: 0.1
        }
        {
            t: 2.0
            type: key
            name: k
            cmd: true
        }
        {t: 2.2, type: key, name: Enter}
        {t: 3.0, type: type, text: " "}
        {t: 4.0, type: key, name: Tab}
        {t: 5.0, type: mark, name: end}
    ]
    assert equal (script-captions $recorded) [
        {t: 1.0, text: "type: a"}
        {t: 1.1, text: "type: ab"}
        {t: 2.0, text: "⌘K  ↩ Return"}
        {t: 3.0, text: "␣ Space"}
        {t: 4.0, text: "⇥ Tab"}
    ]
    let clip = {
        page: extensions
        name: notes
        extension: x
        fixture: y.json
        clip: true
    }
    assert equal (image-paths $clip | first) docs/images/extensions/notes-light.webp

    let hero = {
        page: home
        name: hero
        themes: [dark]
        video: true
        steps: [
            {type: "ma"}
        ]
    }
    assert equal (image-paths $hero) [
        docs/images/home/hero-dark.mp4
        docs/images/home/hero-dark-poster.webp
    ]
    let page = {
        path: website/src/pages/Home.svelte
        text: `<Clip src="/docs/images/home/hero-dark.mp4" poster="/docs/images/home/hero-dark-poster.webp" />`
    }
    assert equal (embedded $page.path $page.text) [
        docs/images/home/hero-dark.mp4
        docs/images/home/hero-dark-poster.webp
    ]
    let hero_files = image-paths $hero
    assert equal (
        problems {images: [], clips: [$hero]} $hero_files [$page] {|path| $path in $hero_files }
    ) []

    let glass = {
        clips: [
            ($hero | insert glass true)
            ($clip | update name other)
        ]
    }
    assert equal (glass-problems $glass {"home/hero": {macos: "26.3"}}) []
    assert equal (glass-problems $glass {"extensions/other": {macos: "26.3"}}) ["glass.json lists extensions/other, which is not a `glass = true` clip"]

    let grid = {
        page: readme
        name: grid
        recipe: grid
        themes: [dark]
        columns: 2
        width: 100
        sources: [home/hero calendar/upcoming]
        composite: true
    }
    let both = {
        images: [$calendar]
        clips: [$hero]
        composites: [$grid]
    }
    assert equal (image-paths $grid) [docs/images/readme/grid-dark.webp]
    assert equal (composite-sources $grid $both dark) [
        docs/images/home/hero-dark-poster.webp
        docs/images/calendar/upcoming-dark.png
    ]
    assert equal (composite-problems $grid $both) []
    let readme = {path: README.md, text: '<img src="./docs/images/readme/grid-dark.webp">'}
    assert equal (embedded $readme.path $readme.text) [docs/images/readme/grid-dark.webp]
    let grid_files = image-paths $calendar | append $hero_files | append (image-paths $grid)
    let calendar_doc = {
        path: docs/features/calendar.md
        text: (picture "../" calendar/upcoming)
    }
    assert equal (
        problems $both $grid_files [$calendar_doc $page $readme] {|path| $path in ($grid_files | append $SOURCES) }
    ) []
    assert (
        problems $both ($grid_files | drop) [$calendar_doc $page $readme] {|path| $path in ($grid_files | append $SOURCES) }
        | any { str contains "docs/images/readme/grid-dark.webp is missing" }
    )
    assert equal (composite-problems ($grid | update sources [home/gone]) $both) [
        "readme/grid is drawn from home/gone, which is not an image or clip in images.toml"
    ]
    assert equal (
        composite-problems ($grid | update sources [extensions/notes]) {clips: [$clip]}
    ) [
        "readme/grid is drawn from extensions/notes, an animated clip with no still"
    ]
    assert equal (composite-problems ($grid | update themes [light]) $both) ["readme/grid is drawn in light from home/hero, which has no light file"]
    assert equal (composite-problems ($grid | update recipe mosaic) $both) ["readme/grid names the recipe `mosaic`; the only one is `grid`"]
}
