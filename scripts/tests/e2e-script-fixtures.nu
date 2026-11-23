use std/assert
use ../e2e-script-fixtures.nu [prepare-scripts cleanup-scripts script-processes-stopped]

def main [] {
    let root = mktemp --directory | path expand
    let failure = try {
        prepare-scripts $root
        let directory = $root | path join config dupin scripts
        let output = $root | path join script-fixtures
        assert not ($output | path join argv.json | path exists)
        for name in [echo stream failure] {
            assert ($directory | path join $name | path exists)
        }
        let again = try {
            prepare-scripts $root
            null
        } catch {|error|
            $error.msg
        }
        assert ($again == 'Script fixtures already exist')
        assert (script-processes-stopped $root)

        let bin = $root | path join bin
        mkdir $bin
        '#!/usr/bin/env nu
def --wrapped main [...args: string] {
    if $env.FIXTURE_PS_MODE == gone { exit 1 }
    if "command=" in $args { print "/foreign/process" } else if "pgid=" in $args { print "999999" } else { print $env.FIXTURE_PS_MODE }
}
' | save ($bin | path join ps)
        ^chmod +x ($bin | path join ps)
        {parent: 999999, child: 999998} | to json | save ($output | path join processes.json)
        with-env {PATH: ($env.PATH | prepend $bin)} {
            with-env {FIXTURE_PS_MODE: Z} { assert (script-processes-stopped $root) }
            with-env {FIXTURE_PS_MODE: S} {
                assert not (script-processes-stopped $root)
                let rejected = try { cleanup-scripts $root; null } catch {|error| $error.msg }
                assert ($rejected == 'Refusing to signal a process outside the script fixture group')
                assert ($directory | path exists)
            }
            with-env {FIXTURE_PS_MODE: gone} { cleanup-scripts $root }
        }
        assert not ($directory | path exists)
        assert ($output | path join processes.json | path exists)
        null
    } catch {|error| $error }
    rm --recursive $root
    if $failure != null { error make $failure }
    print 'Script fixture preparation and cleanup ownership checks passed'
}
