#!/usr/bin/env nix
#! nix shell --inputs-from .. nixpkgs#nushell --command nu
# Drive a built Dupin.app through its accessibility tree with agent-desktop:
# hotkey → panel → type "Calculator" → ↩ → Calculator is running.
# Snapshots hold whatever the panel lists, so run this only on a disposable machine.

# Run the scenario against `app`, keeping every agent-desktop reply, snapshot, screenshot and
# Dupin's log in `out`.
def "main run" [app: path, --out: path = 'target/e2e'] {
    mkdir $out
    let out = $out | path expand
    let app = $app | path expand

    for command in [version status] {
        ad $out $command $command | to json | print
    }
    # The runner image grants Runner.Listener, the app that launches every step, so
    # agent-desktop and screencapture inherit Accessibility, PostEvent and Screen Recording.
    let denied = ad $out permissions permissions | get data | transpose permission grant
        | where permission in [accessibility screen_recording] and grant.state != granted
    if ($denied | is-not-empty) {
        error make {msg: $"not granted: ($denied.permission | str join ', ')"}
    }

    # Spotlight holds ⌘Space on a fresh macOS.
    let config = $out | path join config
    mkdir ($config | path join dupin)
    'hotkey = "alt+space"' | save --force ($config | path join dupin config.toml)
    let log = $out | path join dupin.log
    let dupin = job spawn {
        with-env {XDG_CONFIG_HOME: $config, DUPIN_TRACE: '1'} {
            ^($app | path join Contents MacOS dupin) o+e> $log
        }
    }

    poll $out launched 30 {|r| $r.ok and (mentions $r Dupin) } list-apps
    snap $out launched

    ad $out press-hotkey press alt+space | ignore
    let field = poll $out query-field 10 {|r| $r.ok and $r.data?.match?.ref_id? != null } find --app Dupin --role textfield --first
    snap $out summoned

    # Rune's text field has no settable AX value, so type through the focused field.
    ad $out type --headed type $field.data.match.ref_id Calculator | ignore
    poll $out result 10 {|r| $r.ok and $r.data?.match?.ref_id? != null } find --app Dupin --text Calculator --first
    snap $out results

    let before = ad $out before-return list-apps
    if (mentions $before Calculator) {
        error make {msg: 'Calculator was running before ↩, so its launch would prove nothing'}
    }
    ad $out press-return press return | ignore
    poll $out opened 15 {|r| $r.ok and (mentions $r Calculator) } list-apps
    snap $out opened

    job kill $dupin
    print 'e2e: the panel opened Calculator'
}

# Run agent-desktop, keep its reply in <out>/<name>.json, and return it parsed.
def --wrapped ad [out: path, name: string, ...args: string]: nothing -> record {
    let result = ^agent-desktop ...$args | complete
    $result.stdout | save --force ($out | path join $'($name).json')
    $result.stderr | save --append ($out | path join agent-desktop.stderr)
    try { $result.stdout | from json } catch { {ok: false} }
}

# Retry an agent-desktop command once a second until `done` accepts its reply.
def --wrapped poll [out: path, name: string, tries: int, done: closure, ...args: string] {
    mut reply = {ok: false}
    for attempt in 1..$tries {
        $reply = ad $out $name ...$args
        if (do $done $reply) { return $reply }
        sleep 1sec
    }
    error make {msg: $"timed out waiting for ($name): ($reply | to json --raw)"}
}

def mentions [reply: record, name: string]: nothing -> bool {
    $reply.data? | to json --raw | str contains $'"($name)"'
}

def snap [out: path, name: string] {
    ad $out $name snapshot --app Dupin | ignore
    ^screencapture -x ($out | path join $'($name).png') | complete | ignore
}

def main [] {
    print 'usage: scripts/e2e.nu run <Dupin.app> [--out <dir>]'
}
