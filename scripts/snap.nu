#!/usr/bin/env nu

use headless-config.nu scenario-env

# Render each headless scenario in scripts/headless to <out>/<name>.png.
#
# A scenario with a `<name>.toml` beside it runs with that file as its config.toml, from a
# temporary config directory; the others use yours.
#
# The renders show real results from this Mac (file paths included), so they stay out of the
# repository. `--realtime` paces each run on the wall clock: results come from real providers
# on worker threads (Spotlight, the app scan, icon rasterisation), which Rune's virtual
# headless clock would outrun. The panel fits its content height, which `--snap` doesn't
# follow, so each run first reads the panel's height from the element tree.
# The clipboard history never reads this Mac's clipboard headless; it shows the synthetic
# entries in scripts/fixtures/clipboard.json instead.
def main [
    --bin: path = "target/debug/dupin" # A debug build, or a release build with Rune's `tooling` feature
    --out: path = "target/screenshots"
]: nothing -> table {
    let root = $env.FILE_PWD | path dirname
    cd $root
    $env.DUPIN_HEADLESS_CLIPBOARD = $root | path join scripts/fixtures/clipboard.json
    mkdir $out
    let tmp = mktemp --directory
    let config = $tmp | path join config
    cp --recursive scripts/headless/config $config
    let shots = with-env {XDG_CONFIG_HOME: $config, XDG_DATA_HOME: ($tmp | path join data), HOME: $tmp} { glob scripts/headless/*.json | sort | each {|script|
        let name = $script | path parse | get stem
        let tree = $tmp | path join $"($name).json"
        let config = scenario-env $script $tmp
        with-env $config {
            headless $bin --script $script --realtime --tree $tree --at mark:end
        }
        let height = open $tree | panel-height
        let png = $out | path join $"($name).png"
        with-env $config {
            headless $bin --script $script --realtime --snap $png --at mark:end --scale 2 --size $"680x($height)"
        }
        {name: $name, png: ($png | path expand), height: $height}
    } }
    rm --recursive $tmp
    $shots
}

# The panel's height in points, rounded up, from a `--tree` dump.
def panel-height []: record -> int {
    let panel = $in.elements | where key? == "panel" and rect != null
    if ($panel | is-empty) {
        error make {msg: "the tree has no laid-out element keyed `panel`"}
    }
    $panel | first | get rect.3 | math ceil
}

# Run Dupin headless, quietly unless it fails.
def --wrapped headless [bin: path, ...args: string]: nothing -> nothing {
    let result = ^$bin ...$args | complete
    if $result.exit_code != 0 {
        error make {msg: $"($bin) ($args | str join ' ') failed:\n($result.stderr)"}
    }
}
