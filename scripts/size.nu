#!/usr/bin/env nu

# How big Dupin.app is, and how a pull request changes that.
#
# `measure` writes the sizes of a built bundle as JSON; CI uploads it from every build as the
# `size` artifact. `report` renders a pull request's sizes against those of the main build it
# merged onto, as the Markdown of the Size comment workflow's comment. It reports growth and
# never fails on it. See docs/development.md, "App size".

use main-build.nu find-main-artifact

def main []: nothing -> nothing {
    help main
}

# Each figure, in the order the report lists them. The labels live here rather than in the
# measured JSON, which a pull request's own code writes.
export const METRICS = [
    [key, label];
    [zip, "Dupin.zip, as distributed"]
    [app, "Dupin.app"]
    [binary, "`Contents/MacOS/dupin`"]
    [extensions, "Bundled extensions"]
    [notices, "`ThirdPartyNotices.txt`"]
    [wordlist, "EFF word list"]
    [icons, "Icons"]
    [other, "Everything else"]
]

# Growth the report marks: more than either of these.
export const NOTABLE_PERCENT = 1
export const NOTABLE_BYTES = 102400
const MARKER = "<!-- dupin-size -->"

# Measure a built bundle and write its sizes, in bytes, as JSON.
def "main measure" [
    app: path # A Dupin.app, such as result/Applications/Dupin.app
    --commit: string = "" # The commit it was built from
    --out: path # Where to write the JSON; printed when absent
]: nothing -> nothing {
    let measured = parts $app
    | update sizes { insert zip (zip-size $app) }
    | insert commit $commit
    if $out == null {
        print ($measured | to json)
    } else {
        mkdir ($out | path dirname)
        $measured | to json | save --force $out
    }
}

# Render the sizes in `head` against main's as Markdown. With `--repo`, main's are those of the
# CI run on the main commit `head` merged onto, or else the latest main run that measured any.
def "main report" [
    head: path # `measure`'s JSON
    --repo: string # The GitHub repository whose main build to compare with
    --branch: string = "main"
    --commit: string # The pull request's head, to name in the heading
    --run-url: string # The CI run that measured `head`
]: nothing -> string {
    let measured = open $head | checked
    let baseline = if $repo == null { null } else {
        find-baseline $repo $measured.commit $branch
    }
    render $measured $baseline {
        commit: ($commit | default $measured.commit)
        run_url: $run_url
    }
}

# The size of every file in the bundle, by the part of it the report lists.
export def parts [app: path]: nothing -> record {
    let files = do {
        cd $app
        ls --all ("**/*" | into glob)
        | where type != dir
        | each {|file| {path: ($file.name | path split | str join "/"), bytes: ($file.size | into int)} }
    }
    let total = {|rows|
        $rows | get bytes | reduce --fold 0 {|bytes, sum| $sum + $bytes }
    }
    let classified = $files | insert part {|file| part-of $file.path }
    let sizes = $METRICS.key
    | where $it not-in [zip app]
    | reduce --fold {} {|part, sizes|
        $sizes | insert $part (do $total ($classified | where part == $part))
    }
    let extensions = $classified
    | where part == extensions
    | insert name {|file| $file.path | split row "/" | get 3 }
    | group-by name
    | items {|name, rows| {name: $name, bytes: (do $total $rows)} }
    | reduce --fold {} {|extension, all| $all | insert $extension.name $extension.bytes }
    {
        sizes: ($sizes | insert app (do $total $files))
        extensions: $extensions
    }
}

def part-of [path: string]: nothing -> string {
    let resources = "Contents/Resources/"
    if $path == "Contents/MacOS/dupin" {
        "binary"
    } else if ($path | str starts-with $"($resources)extensions/") {
        "extensions"
    } else if $path == $"($resources)ThirdPartyNotices.txt" {
        "notices"
    } else if $path == $"($resources)eff_large_wordlist.txt" {
        "wordlist"
    } else if ($path | str starts-with $resources) and ($path | str ends-with ".icns") {
        "icons"
    } else {
        "other"
    }
}

# The zip the release ships, made the way scripts/package-app.nu makes it.
def zip-size [app: path]: nothing -> int {
    let directory = mktemp --directory
    let zip = $directory | path join Dupin.zip
    ^/usr/bin/ditto -c -k --keepParent $app $zip
    let bytes = ls $zip | get 0.size | into int
    rm --recursive $directory
    $bytes
}

# `measure`'s JSON, holding only what the report reads, each figure a whole number of bytes.
# The report reads a pull request's JSON with a token that can comment, so a name it prints
# must be one a bundled extension could have.
export def checked []: any -> record {
    let measured = $in
    let commit = $measured | get --optional commit | default ""
    if ($commit | describe) != string or not ($commit =~ '^([0-9a-f]{40})?$') {
        error make {msg: "the measured commit is not a full SHA"}
    }
    let bytes = {|value, what|
        if ($value | describe) != int or $value < 0 {
            error make {msg: $"($what) is not a size in bytes"}
        }
        $value
    }
    let sizes = $METRICS.key | reduce --fold {} {|key, sizes|
        $sizes | insert $key (do $bytes ($measured | get --optional sizes | get --optional $key) $key)
    }
    let extensions = $measured | get --optional extensions | default {}
    if ($extensions | describe | str starts-with record) == false {
        error make {msg: "extensions is not a record"}
    }
    let extensions = $extensions
    | items {|name, value|
        if not ($name =~ '^[a-z0-9][a-z0-9-]{0,63}$') {
            error make {msg: "an extension's name is not a package folder's"}
        }
        {name: $name, bytes: (do $bytes $value $name)}
    }
    | reduce --fold {} {|extension, all| $all | insert $extension.name $extension.bytes }
    {commit: $commit, sizes: $sizes, extensions: $extensions}
}

# The main build to compare `commit`'s sizes with (scripts/main-build.nu), with its sizes as
# `measured`. Nothing when there is none.
export def find-baseline [repo: string, commit: string, branch: string]: nothing -> any {
    find-main-artifact $repo $commit $branch size size.json {|file| open $file | checked }
    | if $in == null { null } else { rename --column {data: measured} }
}

# The comment's Markdown. `baseline` is `find-baseline`'s record, or nothing.
export def render [measured: record, baseline, context: record]: nothing -> string {
    let short = {|commit|
        $commit | str substring 0..6
    }
    let base = $baseline | get --optional measured
    let against = if $baseline == null {
        "No main build has measured the app yet, so there is nothing to compare with."
    } else if $baseline.merge_base {
        $"Against main at [(do $short $baseline.commit)]\(($baseline.url)\), which this build merged onto."
    } else if $baseline.base != "" {
        $"Against main at [(do $short $baseline.commit)]\(($baseline.url)\), the latest main build measured; main at (do $short $baseline.base), which this build merged onto, has no measurement."
    } else {
        $"Against main at [(do $short $baseline.commit)]\(($baseline.url)\), the latest main build measured."
    }
    let rows = $METRICS | each {|metric|
        row $metric.label ($base | get --optional sizes | get --optional $metric.key) ($measured.sizes | get $metric.key)
    }
    let names = $measured.extensions
    | columns
    | append (
        $base
        | get --optional extensions
        | default {}
        | columns
    )
    | uniq
    | sort
    let extensions = $names | each {|name|
        row $"`($name)`" ($base | get --optional extensions | get --optional $name) ($measured.extensions | get --optional $name)
    }
    let header = "| | main | this pull request | change |\n| :-- | --: | --: | --: |"
    let heading = if ($context.commit | default "") == "" { "### App size" } else {
        $"### App size on (do $short $context.commit)"
    }
    let run = if ($context.run_url? | default "") == "" { "" } else { $" [Run]\(($context.run_url)\)." }
    [
        $"($MARKER)\n($heading)"
        $against
        $"($header)\n($rows | str join "\n")"
        $"<details><summary>Bundled extensions</summary>\n\n($header)\n($extensions | str join "\n")\n\n</details>"
        $":warning: marks growth over ($NOTABLE_PERCENT)% or ($NOTABLE_BYTES / 1024 | into int) KiB. Size is informational: it never fails CI. Measured from the `nix build .#dupin` bundle CI builds, the release profile \(fat LTO, one codegen unit, stripped\) that releases sign; they add a Developer ID signature and a notarization ticket.($run)"
    ]
    | str join "\n\n"
    | $"($in)\n"
}

def row [label: string, was, now]: nothing -> string {
    let cell = {|bytes|
        if $bytes == null { "–" } else { human $bytes }
    }
    let change = change $was $now
    let mark = if $change.notable { " :warning:" } else { "" }
    $"| ($label) | (do $cell $was) | (do $cell $now) | ($change.text)($mark) |"
}

# How `now` differs from `was`, in bytes and percent, and whether it is growth to mark.
export def change [was, now]: nothing -> record {
    if $was == null or $now == null {
        return {text: "–", notable: false}
    }
    let delta = $now - $was
    if $delta == 0 {
        return {text: "0 B", notable: false}
    }
    let sign = if $delta > 0 { "+" } else { "-" }
    let bytes = $"($sign)(grouped ($delta | math abs)) B"
    if $was == 0 {
        return {
            text: $bytes
            notable: ($delta > $NOTABLE_BYTES)
        }
    }
    let percent = $delta * 100.0 / $was
    {
        text: $"($bytes) \(($sign)($percent | math abs | into string --decimals 2)%\)"
        notable: ($delta > $NOTABLE_BYTES or $percent > $NOTABLE_PERCENT)
    }
}

export def human [bytes: int]: nothing -> string {
    if $bytes < 1024 {
        $"($bytes) B"
    } else if $bytes < 1048576 {
        $"($bytes / 1024 | into string --decimals 1) KiB"
    } else {
        $"($bytes / 1048576 | into string --decimals 2) MiB"
    }
}

def grouped [n: int]: nothing -> string {
    $n
    | into string
    | split chars
    | reverse
    | chunks 3
    | each { reverse | str join }
    | reverse
    | str join ","
}
