#!/usr/bin/env nu

const ROOT = path self ../..
const FONT_COMMIT = "0eb7e7be6f65e356cfffd0de8caf3e74e4e9437c"
const FONT_URL = $"https://raw.githubusercontent.com/google/fonts/($FONT_COMMIT)/ofl/shipporiminchob1/ShipporiMinchoB1-Bold.ttf"
const OUTPUT = "website/src/fonts/shippori-mincho-b1-bold-ja.woff2"
const RANGE = "website/src/fonts/ja.json"

# Regenerate the Japanese display-font subset from Shippori Mincho B1 Bold (SIL OFL 1.1). The
# subset holds only the characters wrapped in <span lang="ja"> in docs/, so the site ships a few
# hundred bytes rather than a full CJK font. Run it again after adding Japanese text there.
# The U+XXXX of one character, read from its UTF-8 bytes.
def codepoint [char: string]: nothing -> string {
    let bytes = $char | encode hex | split chars | chunks 2 | each { str join | into int --radix 16 }
    let value = if ($bytes | length) == 1 {
        $bytes.0
    } else {
        let lead = $bytes.0 | bits and (255 | bits shr ($bytes | length | $in + 1))
        $bytes | skip 1 | reduce --fold $lead {|byte, acc| ($acc | bits shl 6) | bits or ($byte | bits and 63) }
    }
    $"U+($value | format number | get upperhex | str replace '0x' '')"
}

def main []: nothing -> nothing {
    let glyphs = (
        glob ($ROOT | path join docs/**/*.md)
        | each { open --raw $in | parse --regex '<span lang="ja">(?<text>[^<]*)</span>' | get text }
        | flatten
        | str join
        | split chars
        | where { $in !~ '\s' }
        | uniq
        | sort
        | str join
    )
    if ($glyphs | is-empty) {
        error make {msg: 'no <span lang="ja"> text found in docs/'}
    }

    let work = mktemp --directory
    let source = $work | path join source.ttf
    let output = $ROOT | path join $OUTPUT
    http get --raw $FONT_URL | save --force --raw $source
    let python = '(builtins.getFlake "nixpkgs").legacyPackages.${builtins.currentSystem}.python3.withPackages (p: [ p.fonttools p.brotli ])'
    (
        nix shell --impure --expr $python --command
        pyftsubset $source $"--text=($glyphs)" --flavor=woff2 --layout-features= --no-hinting
        --desubroutinize $"--output-file=($output)"
    )
    rm --recursive --force $work

    let unicode_range = $glyphs | split chars | each { codepoint $in } | str join ","
    {text: $glyphs, unicodeRange: $unicode_range} | to json --indent 2 | save --force ($ROOT | path join $RANGE)

    print $"($glyphs) -> ($OUTPUT): (open --raw $output | bytes length) bytes, ($unicode_range)"
}
