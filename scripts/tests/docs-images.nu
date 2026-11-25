use std/assert
use ../docs-images.nu [embedded image-paths problems]

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
}
