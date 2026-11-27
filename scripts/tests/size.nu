use std/assert
use ../size.nu [checked change human parts render]

def sizes [binary: int]: nothing -> record {
    {
        sizes: {
            zip: 1000
            app: ($binary + 900)
            binary: $binary
            extensions: 500
            notices: 300
            wordlist: 60
            icons: 30
            other: 10
        }
        extensions: {calculator: 200, emoji: 300}
        commit: ("a" | fill --character a --width 40)
    }
}

def main [] {

    # Every file counts once, hidden ones too, by the part of the bundle it is in.
    let app = mktemp --directory | path join Dupin.app
    let resources = $app | path join Contents/Resources
    mkdir ($app | path join Contents/MacOS) ($resources | path join extensions/emoji/dist)
    "12345678" | save ($app | path join Contents/MacOS/dupin)
    "plist" | save ($app | path join Contents/Info.plist)
    "abc" | save ($resources | path join ThirdPartyNotices.txt)
    "ab" | save ($resources | path join eff_large_wordlist.txt)
    "icon" | save ($resources | path join dupin.icns)
    "{}" | save ($resources | path join extensions/emoji/package.json)
    "x" | save ($resources | path join extensions/emoji/dist/.hidden)
    let measured = parts $app
    rm --recursive ($app | path dirname)
    assert equal $measured.sizes {
        binary: 8
        extensions: 3
        notices: 3
        wordlist: 2
        icons: 4
        other: 5
        app: 25
    }
    assert equal $measured.extensions {emoji: 3}

    # Growth is marked over 1% or 100 KiB; shrinking never is.
    assert equal (change 1000 1010) {text: "+10 B (+1.00%)", notable: false}
    assert equal (change 1000 1011) {text: "+11 B (+1.10%)", notable: true}
    assert equal (change 20000000 20102400) {text: "+102,400 B (+0.51%)", notable: false}
    assert equal (change 20000000 20102401) {text: "+102,401 B (+0.51%)", notable: true}
    assert equal (change 2000 1000) {text: "-1,000 B (-50.00%)", notable: false}
    assert equal (change 5 5) {text: "0 B", notable: false}
    assert equal (change null 5) {text: "–", notable: false}
    assert equal (human 1023) "1023 B"
    assert equal (human 20879056) "19.91 MiB"

    # The report reads a pull request's JSON with a token that can comment.
    assert equal (sizes 100 | checked) (sizes 100)
    assert error { sizes 100 | update sizes.binary "100" | checked }
    assert error { sizes 100 | update sizes.binary (-1) | checked }
    assert error { sizes 100 | reject sizes.zip | checked }
    assert error { sizes 100 | update commit "main; rm -rf /" | checked }
    assert error { sizes 100 | update extensions { insert "[x](https://example.com)" 1 } | checked }

    let base = sizes 20000000
    let head = sizes 20200000 | update extensions { insert dictionary 50 }
    let compared = render $head {
        measured: $base
        commit: ("b" | fill --character b --width 40)
        url: "https://example.com/run"
        merge_base: true
        base: ("b" | fill --character b --width 40)
    } {commit: ("c" | fill --character c --width 40), run_url: "https://example.com/pr"}
    let lines = $compared | lines
    assert equal $lines.0 "<!-- dupin-size -->"
    assert equal $lines.1 "### App size on ccccccc"
    assert (
        $compared
        | str contains "Against main at [bbbbbbb](https://example.com/run), which this build merged onto."
    )
    assert (
        "| `Contents/MacOS/dupin` | 19.07 MiB | 19.26 MiB | +200,000 B (+1.00%) :warning: |" in $lines
    )
    assert ("| Dupin.zip, as distributed | 1000 B | 1000 B | 0 B |" in $lines)
    assert ("| `dictionary` | – | 50 B | – |" in $lines)

    let alone = render (sizes 100) null {commit: "", run_url: null} | lines
    assert equal $alone.1 "### App size"
    assert (
        "No main build has measured the app yet, so there is nothing to compare with." in $alone
    )
    assert ("| `Contents/MacOS/dupin` | – | 100 B | – |" in $alone)
}
