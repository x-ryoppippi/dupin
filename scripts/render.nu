#!/usr/bin/env nu

use headless-config.nu [scenario-env panel-height headless]

# Render each headless scenario in scripts/headless to <out>/<name>.png.
#
# Each scenario runs with temporary HOME, XDG_DATA_HOME and XDG_CONFIG_HOME directories.
# The config comes from scripts/headless/config unless a matching `<name>.toml` supplies it.
#
# The renders show real results from this Mac (file paths included), so they stay out of the
# repository. `--realtime` paces each run on the wall clock: results come from real providers
# on worker threads (Spotlight, the app scan, icon rasterisation), which Rune's virtual
# headless clock would outrun. The panel fits its content height, which `--snap` doesn't
# follow, so each run first reads the panel's height from the element tree.
# The clipboard history never reads this Mac's clipboard headless; it shows the synthetic
# entries in scripts/fixtures/clipboard.json instead.
def main [
    --bin: path = "target/debug/dupin" # Any build of Dupin, or the app's Contents/MacOS/dupin
    --out: path = "target/screenshots"
    --theme: string # `light` or `dark`; the Mac's appearance when omitted
    --only: string # Render only the scenarios whose name contains this
]: nothing -> table {
    let root = $env.FILE_PWD | path dirname
    cd $root
    $env.DUPIN_HEADLESS_CLIPBOARD = $root | path join scripts/fixtures/clipboard.json
    mkdir $out
    let tmp = mktemp --directory
    let config = $tmp | path join config
    cp --recursive scripts/headless/config $config
    let appearance = if $theme == null { [] } else { [--theme $theme] }
    let suffix = if $theme == null { "" } else { $"-($theme)" }
    let scripts = glob scripts/headless/*.json | sort | where {|script|
        $only == null or ($script | path parse | get stem | str contains $only)
    }
    let shots = with-env {XDG_CONFIG_HOME: $config, XDG_DATA_HOME: ($tmp | path join data), HOME: $tmp} { $scripts | each {|script|
        let name = $script | path parse | get stem
        let tree = $tmp | path join $"($name).json"
        let config = scenario-env $script $tmp
        with-env $config {
            headless $bin --script $script --realtime ...$appearance --tree $tree --at mark:end
        }
        let height = open $tree | panel-height
        let png = $out | path join $"($name)($suffix).png"
        with-env $config {
            headless $bin --script $script --realtime ...$appearance --snap $png --at mark:end --scale 2 --size $"680x($height)"
        }
        {name: $name, png: ($png | path expand), height: $height}
    } }
    rm --recursive $tmp
    $shots
}
