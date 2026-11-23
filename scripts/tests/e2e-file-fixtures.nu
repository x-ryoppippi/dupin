use std/assert
use ../e2e-file-fixtures.nu [viewer-running readable-trash]

def main [] {
    let root = mktemp --directory | path expand
    let failure = try {
        let bin = $root | path join bin
        let home = $root | path join home
        mkdir $bin ($home | path join .Trash)
        '#!/usr/bin/env nu
def --wrapped main [...args: string] { exit ($env.FIXTURE_PGREP_EXIT | into int) }
' | save ($bin | path join pgrep)
        ^chmod +x ($bin | path join pgrep)
        with-env {HOME: $home, PATH: ($env.PATH | prepend $bin)} {
            readable-trash
            with-env {FIXTURE_PGREP_EXIT: '0'} { assert (viewer-running) }
            with-env {FIXTURE_PGREP_EXIT: '1'} { assert not (viewer-running) }
            let process_failure = with-env {FIXTURE_PGREP_EXIT: '2'} {
                try { viewer-running; null } catch {|error| $error.msg }
            }
            assert ($process_failure == 'Could not check the fixture viewer process')
            rm --recursive ($home | path join .Trash)
            let trash_failure = try { readable-trash; null } catch {|error| $error.msg }
            assert ($trash_failure | str starts-with 'Cannot read the disposable runner Trash')
        }
        null
    } catch {|error| $error }
    rm --recursive $root
    if $failure != null { error make $failure }
    print 'E2E fixture process and Trash access checks passed'
}
