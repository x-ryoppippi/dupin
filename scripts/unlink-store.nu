# Point the binary's /nix/store libraries at the copies macOS ships, so the bundle runs
# without Nix. A library macOS doesn't ship stops the release instead.
export def main [binary: path]: nothing -> nothing {
    let system = [libiconv.2.dylib]
    let result = ^/usr/bin/otool -L $binary | complete
    if $result.exit_code != 0 {
        error make {msg: $result.stderr}
    }
    let linked = $result.stdout
    | lines
    | skip 1
    | str trim
    | parse "{lib} (compatibility {rest}"
    | get lib
    | str trim
    | where { str starts-with /nix/store/ }
    for lib in $linked {
        let name = $lib | path basename
        if $name not-in $system {
            error make {msg: $"($binary) links ($lib), which macOS doesn't ship"}
        }
        let changed = ^/usr/bin/install_name_tool -change $lib $"/usr/lib/($name)" $binary | complete
        if $changed.exit_code != 0 {
            error make {msg: $changed.stderr}
        }
    }
}
