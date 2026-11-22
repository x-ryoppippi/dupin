#!/usr/bin/env nix
#! nix shell --inputs-from .. nixpkgs#nushell --command nu
# Drive a built Dupin.app through its accessibility tree with agent-desktop:
# hotkey → panel → type "calculator" → ↩ → Calculator is running, then Settings' pages. The
# panel draws the built-in backdrop shader, so the screenshots show it.
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
    # agent-desktop, osascript and screencapture inherit Accessibility, Automation, PostEvent and
    # Screen Recording.
    let denied = ad $out permissions permissions
    | get data
    | transpose permission grant
    | where permission in [accessibility automation screen_recording] and grant.state != granted
    if ($denied | is-not-empty) {
        error make {msg: $"not granted: ($denied.permission | str join ', ')"}
    }

    # Spotlight holds ⌘Space on a fresh macOS.
    let config = $out | path join config
    mkdir ($config | path join dupin)
    "hotkey = \"alt+space\"\nbackdrop = \"flow\"\n" | save --force ($config | path join dupin config.toml)
    let log = $out | path join dupin.log
    let dupin = job spawn {
        with-env {XDG_CONFIG_HOME: $config, DUPIN_TRACE: '1'} {
            ^($app | path join Contents MacOS dupin) o+e> $log
        }
    }

    # Dupin is running well before its on_ready callback registers the hotkey.
    wait-for-log $log 'dupin-trace: hotkey alt+space registered'
    snap $out launched

    ad $out press-hotkey press alt+space | ignore
    let focused = {|r| $r.ok and 'focused' in ($r.data?.match?.states? | default []) }
    poll $out field $focused find --app Dupin --role textfield --first
    # Past the backdrop's fade-in, so the screenshot shows the shader.
    sleep 500ms
    snap $out summoned

    # `type` needs a settable AX value or the main window, and the field has neither: the
    # panel never activates, so it is key but not main. Press keys as a person would, from one
    # process: one `agent-desktop press` per key lost keys before they reached Dupin.
    ^osascript -e 'tell application "System Events" to keystroke "calculator"'
    snap $out typed
    # Fuzzy matching would still find Calculator from a query missing keys, so check every key
    # reached the field.
    let typed = {|r| $r.ok and $r.data?.match?.value? == 'calculator' }
    poll --tries 5 $out typed-value $typed find --app Dupin --role textfield --first
    # A row's name is its title and subtitle. `--text` would also match the typed field value
    # and every ancestor holding it.
    let row = 'Calculator /System/Applications'
    let found = {|r| $r.ok and $r.data?.match? != null }
    poll $out result $found find --app Dupin --name $row --exact --first
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

    # Calculator can still be coming forward; summoned before then, the panel would lose focus
    # to it and hide before ⌘, arrives.
    wait-frontmost Calculator
    ad $out press-hotkey-again press alt+space | ignore
    poll $out field-again $focused find --app Dupin --role textfield --first
    ^osascript -e 'tell application "System Events" to keystroke "," using command down'
    let page = {|r| $r.ok and $r.data?.match?.ref_id? != null }
    let appearance = (poll
        $out
        settings
        $page
        find
        --app
        Dupin
        --name
        Appearance
        --exact
        --first
    )
    wait-frontmost Dupin
    snap $out settings-general
    ad $out open-appearance click $appearance.data.match.ref_id | ignore
    (poll
        $out
        appearance-page
        $found
        find
        --app
        Dupin
        --name
        Backdrop
        --exact
        --first
    )
    wait-frontmost Dupin
    snap $out settings-appearance

    job kill $dupin
    # A shader that fails falls back to the built-in one with a warning, which the
    # screenshots alone wouldn't show.
    if (open --raw $log | lines | any {|line| $line =~ '^dupin: backdrop (shader|pipeline)' }) {
        error make {msg: $"the backdrop shader failed; see ($log)"}
    }
    print 'e2e: the panel opened Calculator, and Settings showed its pages'
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
    let shot = ^screencapture -x ($out | path join $'($name).png') | complete
    if $shot.exit_code != 0 {
        error make {msg: $"screencapture failed for ($name): ($shot.stderr)"}
    }
}

# Wait until `app` is the frontmost application, so a screenshot shows its window.
def wait-frontmost [app: string, --tries: int = 20] {
    let script = 'tell application "System Events" to get name of first application process whose frontmost is true'
    mut front = ''
    for attempt in 1..$tries {
        $front = (
            ^osascript -e $script
            | complete
            | get stdout
            | str trim
        )
        if ($front | str lowercase) == ($app | str lowercase) { return }
        sleep 500ms
    }
    error make {msg: $"timed out waiting for ($app) to be frontmost; ($front) is"}
}

def main [] {
    print 'usage: scripts/e2e.nu run <Dupin.app> [--out <dir>]'
}
