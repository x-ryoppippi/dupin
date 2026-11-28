#!/usr/bin/env nu

# The README's hero: the home page's Liquid Glass posters (scripts/docs-glass.nu) in a 2×3 grid.
# Sizes are in pixels at 2x, for an 880-point-wide display on GitHub.

const POSTERS = [
    apps
    files
    dictionary
    in-files
    keywords
    clipboard
]

const COLUMNS = 3
const TILE_WIDTH = 568
const GUTTER = 24
const RADIUS = 24
const OUT = "docs/images/readme/hero.webp"

def main []: nothing -> nothing {
    cd ($env.FILE_PWD | path dirname)
    let scratch = mktemp --directory --tmpdir readme-hero.XXXXXX
    let tiles = $POSTERS | each {|name|
        let tile = $scratch | path join $"($name).png"
        ^magick $"docs/images/home/($name)-dark-poster.webp" -resize $"($TILE_WIDTH)x" $tile
        let height = ^magick identify -format "%h" $tile | into int
        # The gutters stay transparent, so the grid sits on GitHub's light and dark themes alike.
        (
            ^magick $tile
            "(" -size $"($TILE_WIDTH)x($height)" xc:none -fill white
            -draw $"roundrectangle 0,0,($TILE_WIDTH - 1),($height - 1),($RADIUS),($RADIUS)" ")"
            -alpha set -compose DstIn -composite $tile
        )
        $tile
    }
    let rows = $tiles | chunks $COLUMNS | enumerate | each {|row|
        let out = $scratch | path join $"row-($row.index).png"
        ^magick -background none ...$row.item +smush $GUTTER $out
        $out
    }
    mkdir ($OUT | path dirname)
    ^magick -background none ...$rows -smush $GUTTER -quality 90 -define webp:method=6 -define webp:alpha-quality=100 $OUT
    rm --recursive $scratch
    print $"($OUT): (^magick identify -format '%wx%h' $OUT), (ls $OUT | get 0.size)"
}
