#!/usr/bin/env nu

const ROOT = path self ..
const TREES = ["crates" "extensions" "examples/shaders" "sdk"]
const MARKERS = '(?i)copyright\s+(\(c\)|©|[0-9]{4})|\(c\)\s+[0-9]{4}|SPDX-License-Identifier'
const PLACEHOLDER = '^\s*Copyright.*<[a-z ]+>'

# Third-party notices for Dupin.app. See "Third-party code and data" in docs/development.md.
def main []: nothing -> nothing {
    print (help main)
}

# Write the notices: every crate the dupin binary links, with its licence texts as the crate
# ships them, then each entry of packaging/third-party.toml. Fails when a crate's licence isn't
# on deny.toml's allow-list or its text can't be found.
def "main generate" [output: path]: nothing -> nothing {
    let deny = open ($ROOT | path join deny.toml)
    let config = mktemp --tmpdir --suffix .toml
    {
        accepted: $deny.licenses.allow
        targets: $deny.graph.targets
        ignore-dev-dependencies: $deny.graph.exclude-dev
        private: {ignore: $deny.licenses.private.ignore}
    }
    | to toml
    | save --force $config
    let about = (
        ^cargo-about generate --format json --frozen --fail
        --manifest-path ($ROOT | path join crates/dupin/Cargo.toml)
        --config $config
        --threshold $deny.licenses.confidence-threshold
    )
    | from json
    rm $config
    let holders = third-party
    | get crate
    | each {|entry| $entry.names | each {|name| {name: $name, copyright: $entry.copyright} } }
    | flatten
    # cargo-about leaves source_path empty when it fell back to the licence's template.
    let templated = $about.licenses
    | where source_path == null
    | where {|licence| $licence.text | lines | any {|line| $line =~ $PLACEHOLDER } }
    | get used_by
    | flatten
    | get crate.name
    | uniq
    let unrecorded = $templated | where {|name| $name not-in $holders.name }
    let stale = $holders.name | where {|name| $name not-in $templated }
    if ($unrecorded | is-not-empty) or ($stale | is-not-empty) {
        error make --unspanned {
            msg: ([
                ...($unrecorded | each {|name| $"($name) ships no licence text with its copyright holders; record them under [[crate]] in packaging/third-party.toml" })
                ...($stale | each {|name| $"($name) is under [[crate]] in packaging/third-party.toml but its licence text now comes with the crate; remove it" })
            ] | str join "\n")
        }
    }
    let crates = $about.licenses
    | each {|licence|
        let users = $licence.used_by | get crate | uniq-by name version
        let templated = $licence.source_path == null and ($licence.text | lines | any {|line| $line =~ $PLACEHOLDER })
        let text = if $templated {
            let notices = $users | each {|user|
                let lines = $holders | where name == $user.name | first | get copyright
                if ($lines | is-empty) {
                    $"  ($user.name) ($user.version): its licence names no copyright holder"
                } else {
                    $lines | each {|line| $"  ($user.name) ($user.version): ($line)" } | str join "\n"
                }
            }
            [
                "Copyright notices:"
                ...$notices
                ""
                ($licence.text | lines | where {|line| $line !~ $PLACEHOLDER } | str join "\n" | str trim)
            ]
            | str join "\n"
        } else {
            $licence.text | str trim
        }
        [
            (rule $licence.name)
            "Used by:"
            ...($users | each {|user| $"  ($user.name) ($user.version)" })
            ""
            $text
        ]
        | str join "\n"
    }
    let material = materials | each {|entry|
        [
            (rule $entry.name)
            $"Used for: ($entry.use)"
            $"Source: ($entry.source)"
            $"Licence: ($entry.licence)"
            ""
            ...($entry.copyright? | default [])
            ...(if ($entry.copyright? | is-not-empty) { [""] } else { [] })
            (open --raw ($ROOT | path join $entry.text) | str trim)
        ]
        | str join "\n"
    }
    [
        ([
            "Third-party notices for Dupin"
            ""
            "Dupin is free software under the GNU General Public License, version 3 or later; its"
            "source is at https://github.com/x-ryoppippi/dupin. It includes the open-source code"
            "and data below, each under the licence shown with it."
        ] | str join "\n")
        ...$crates
        ...$material
    ]
    | str join "\n\n"
    | $in + "\n"
    | save --force $output
}

# Hold packaging/third-party.toml to the repository: each entry's licence allowed, its text and
# files present, data-licensed files never compiled into the binary, and every file under the
# shipped trees that names a copyright holder or licence other than Dupin's listed by an entry.
def "main check" []: nothing -> nothing {
    let deny = open ($ROOT | path join deny.toml)
    let data_licences = third-party | get data-licences
    let entries = materials
    let errors = $entries
    | each {|entry|
        let licence = if $entry.licence in $deny.licenses.allow or $entry.licence in $data_licences {
            []
        } else {
            [$"($entry.name): ($entry.licence) is on neither deny.toml's allow-list nor data-licences"]
        }
        let missing = [$entry.text ...$entry.files]
        | where {|file| not ($ROOT | path join $file | path exists) }
        | each {|file| $"($entry.name): ($file) doesn't exist" }
        let compiled = if $entry.licence in $data_licences {
            $entry.files | each {|file|
                let name = $file | path basename
                ^rg --files-with-matches --fixed-strings $name ($ROOT | path join crates)
                | complete
                | get stdout
                | lines
                | where {|source| open --raw $source | str contains $"include_" }
                | each {|source| $"($entry.name): ($source) may compile ($file) into the binary; data under ($entry.licence) must ship as its own file" }
            }
            | flatten
        } else {
            []
        }
        [...$licence ...$missing ...$compiled]
    }
    | flatten
    let listed = $entries | get files | flatten | each {|file| $ROOT | path join $file }
    let unlisted = $TREES
    | each {|tree|
        ^rg --files-with-matches --pcre2 $MARKERS ($ROOT | path join $tree)
        | complete
        | get stdout
        | lines
    }
    | flatten
    | where {|file|
        not ($listed | any {|entry| $file == $entry or ($file | str starts-with $"($entry)/") })
    }
    | where {|file| $file | path basename | $in !~ '^LICENSE' }
    | where {|file|
        open --raw $file
        | lines
        | where {|line| $line =~ $MARKERS }
        | any {|line| not ($line =~ 'SPDX-License-Identifier: GPL-3\.0-or-later') }
    }
    | each {|file| $"($file | path relative-to $ROOT) names a copyright holder or licence; list it in packaging/third-party.toml" }
    let all = [...$errors ...$unlisted]
    if ($all | is-not-empty) {
        error make --unspanned {
            msg: ($all | str join "\n")
        }
    }
}

def third-party []: nothing -> record {
    open ($ROOT | path join packaging/third-party.toml)
}

def materials []: nothing -> list<record> {
    third-party | get material
}

def rule [title: string]: nothing -> string {
    $"('' | fill --character '=' --width 78)\n($title)"
}
