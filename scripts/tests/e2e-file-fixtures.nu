use std/assert
use ../e2e-file-fixtures.nu [documents trashed viewer-running]

def main [] {
    let root = mktemp --directory | path expand
    let failure = try {
        let bin = $root | path join bin
        mkdir $bin
        '#!/usr/bin/env nu
def --wrapped main [...args: string] { exit ($env.FIXTURE_PGREP_EXIT | into int) }
' | save ($bin | path join pgrep)
        ^chmod +x ($bin | path join pgrep)
        with-env {PATH: ($env.PATH | prepend $bin)} {
            with-env {FIXTURE_PGREP_EXIT: '0'} { assert (viewer-running) }
            with-env {FIXTURE_PGREP_EXIT: '1'} { assert not (viewer-running) }
            let process_failure = with-env {FIXTURE_PGREP_EXIT: '2'} {
                try { viewer-running; null } catch {|error| $error.msg }
            }
            assert ($process_failure == 'Could not check the fixture viewer process')
        }

        let out = $root | path join out
        let trash = documents $out | path join .Trashes (^id -u | str trim)
        assert equal (trashed $out) []
        mkdir $trash
        for name in ['trash-me.dupine2e' 'trash-me 2.dupine2e' 'open-me.dupine2e'] {
            '' | save ($trash | path join $name)
        }
        assert equal (trashed $out | path basename | sort) ['trash-me 2.dupine2e' 'trash-me.dupine2e']
        null
    } catch {|error| $error }
    rm --recursive $root
    if $failure != null { error make $failure }
    print 'E2E fixture process and Trash location checks passed'
}
