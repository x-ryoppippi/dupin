#!/usr/bin/env nix
#! nix shell --inputs-from .. nixpkgs#nushell --command nu
# Drive a built Dupin.app through its accessibility tree with agent-desktop.
# Snapshots hold whatever the panel lists, so run this only on a disposable machine.

use e2e/driver.nu run-suite
use e2e/launcher.nu launcher-stage
use e2e/snapshots.nu snapshot-stage
use e2e/calculator.nu calculator-stage
use e2e/settings.nu settings-stage
use e2e/system-settings.nu system-pane
use e2e/files.nu file-actions-stage
use e2e/scripts.nu scripts-stage
use e2e/cli.nu cli-stage

# Run the scenario against `app`, keeping each step's last agent-desktop reply, a snapshot and a
# screenshot per stage, and Dupin's log in `out`.
def "main run" [app: path, --out: path = 'target/e2e', --record] {
    run-suite $app $out --record=$record {|out, cli|
        launcher-stage $out
        snapshot-stage $out
        calculator-stage $out
        settings-stage $out
        system-pane $out
        file-actions-stage $out
        scripts-stage $out
        with-env {XDG_RUNTIME_DIR: $cli.runtime, XDG_DATA_HOME: $cli.data} { cli-stage $out $cli.binary }
    }
    print 'e2e: Quick Look, Calculator, Settings, Bluetooth, Open with…, confirmed Trash, CLI picker, Snapshot controls and Script Commands passed'
}

def main [] {
    print 'usage: scripts/e2e.nu run <Dupin.app> [--out <dir>] [--record]'
}
