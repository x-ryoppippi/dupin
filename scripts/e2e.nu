#!/usr/bin/env nix
#! nix shell --inputs-from .. nixpkgs#nushell --command nu
# Drive a built Dupin.app through its accessibility tree with agent-desktop:
# hotkey → panel → type "calculator" → ↩ → Calculator is running.
# Snapshots hold whatever the panel lists, so run this only on a disposable machine.

# Run the scenario against `app`, keeping each step's last agent-desktop reply, a snapshot and a
# screenshot per stage, and Dupin's log in `out`.
def "main run" [app: path, --out: path = 'target/e2e'] {
    mkdir $out
    let out = $out | path expand
    let app = $app | path expand

    for command in [version status] {
        ad $out $command $command | to json | print
    }
    # The runner image grants Runner.Listener, the app that launches every step, so
    # agent-desktop and screencapture inherit Accessibility, PostEvent and Screen Recording.
    let denied = ad $out permissions permissions
    | get data
    | transpose permission grant
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

    poll --tries 30 $out launched {|r| $r.ok and (mentions $r Dupin) } list-apps
    snap $out launched

    let matched = {|r| $r.ok and $r.data?.match?.ref_id? != null }
    # Dupin shows up in list-apps before its on_ready callback registers the hotkey.
    wait-for-log $log 'dupin-trace: hotkey alt+space registered'
    ad $out press-hotkey press alt+space | ignore
    let field = poll $out field $matched find --app Dupin --role textfield --first
    snap $out summoned

    if 'focused' not-in $field.data.match.states {
        error make {msg: 'the query field is not focused after the hotkey'}
    }
    # `type` needs a settable AX value or the main window, and the field has neither: the
    # panel never activates, so it is key but not main. Press keys as a person would.
    for key in ('calculator' | split chars | enumerate) {
        ad $out $'type-($key.index)' press $key.item | ignore
    }
    snap $out typed
    # Fuzzy matching would still find Calculator from a query missing keys, so check every key
    # reached the field.
    let typed = {|r| $r.ok and $r.data?.match?.value? == 'calculator' }
    poll --tries 5 $out typed-value $typed find --app Dupin --role textfield --first
    # Match a node named Calculator. `--text` also matches the typed field value and every
    # ancestor holding it, and a result row need not carry a ref, since ↩ opens the selection.
    let found = {|r| $r.ok and $r.data?.match? != null }
    poll $out result $found find --app Dupin --name Calculator --first
    snap $out results

    let before = poll $out before-return {|r| $r.ok } list-apps
    if (mentions $before Calculator) {
        error make {
            msg: 'Calculator was running before ↩, so its launch would prove nothing'
        }
    }
    ad $out press-return press return | ignore
    poll --tries 15 $out opened {|r| $r.ok and (mentions $r Calculator) } list-apps
    snap $out opened

    job kill $dupin
    print 'e2e: the panel opened Calculator'
}

# Run agent-desktop, keep its reply in <out>/<name>.json, and return it parsed.
def --wrapped ad [out: path, name: string, ...args: string]: nothing -> record {
    let result = ^agent-desktop ...$args | complete
    $result.stdout | save --force ($out | path join $'($name).json')
    $result.stderr | save --append ($out | path join agent-desktop.stderr)
    try {
        $result.stdout | from json
    } catch {
        {ok: false}
    }
}

# Retry an agent-desktop command once a second until `done` accepts its reply.
def --wrapped poll [
    out: path
    name: string
    done: closure
    --tries: int = 10
    ...args: string
] {
    mut reply = {ok: false}
    for attempt in 1..$tries {
        $reply = ad $out $name ...$args
        if (do $done $reply) { return $reply }
        sleep 1sec
    }
    error make {msg: $"timed out waiting for ($name): ($reply | to json --raw)"}
}

def wait-for-log [log: path, line: string, --tries: int = 30] {
    for attempt in 1..$tries {
        if ($log | path exists) and (open --raw $log | str contains $line) { return }
        sleep 1sec
    }
    error make {msg: $"timed out waiting for ($line) in ($log)"}
}

def mentions [reply: record, name: string]: nothing -> bool {
    $reply.data? | to json --raw | str contains $'"($name)"'
}

def snap [out: path, name: string] {
    ad $out $'($name)-snapshot' snapshot --app Dupin | ignore
    ^screencapture -x ($out | path join $'($name).png') | complete | ignore
}

def main [] {
    print 'usage: scripts/e2e.nu run <Dupin.app> [--out <dir>]'
}
