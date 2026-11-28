use ../e2e-recording.nu record
use ../e2e-script-fixtures.nu [prepare-scripts cleanup-scripts]
use ../e2e-file-fixtures.nu [prepare cleanup viewer-running]
use keys.nu key-script

# agent-desktop takes every process run from Dupin.app/Contents/MacOS for an instance of the app
# named Dupin, the extension host and a waiting CLI included. Only the resident launcher is an
# application to AppKit, so only it answers to the bundle ID.
export const DUPIN = 'app.dupin.Dupin'

export def run-suite [
    app: path
    out: path
    suite: closure
    --record
] {
    mkdir $out
    let out = $out | path expand
    let app = $app | path expand
    let bundle_id = ^plutil -extract CFBundleIdentifier raw -o - ($app | path join Contents Info.plist) | str trim
    if $bundle_id != $DUPIN {
        error make {msg: $"($app) is ($bundle_id), not ($DUPIN)"}
    }

    for command in [version status] {
        ad $out $command $command | to json | print
    }
    # The runner image grants Runner.Listener, the app that launches every step, so
    # agent-desktop, osascript, screencapture and ffmpeg inherit Accessibility, Automation, PostEvent and
    # Screen Recording.
    let denied = ad $out permissions permissions
    | get data
    | transpose permission grant
    | where permission in [accessibility automation screen_recording] and grant.state != granted
    if ($denied | is-not-empty) {
        error make {msg: $"not granted: ($denied.permission | str join ', ')"}
    }

    if ($out | path join script-fixtures | path exists) or ($out | path join config dupin scripts | path exists) {
        error make {msg: 'Script fixtures already exist'}
    }
    if (viewer-running) {
        error make {msg: 'Cannot establish ownership of the file-action viewer'}
    }
    if ($out | path join fixtures | path exists) {
        error make {msg: 'File-action fixtures already exist'}
    }
    try { prepare $out } catch {|failure|
        try { cleanup $out }
        error make $failure
    }

    # Spotlight holds ⌘Space on a fresh macOS.
    let config = $out | path join config
    mkdir ($config | path join dupin)
    "hotkey = \"alt+space\"\nbackdrop = \"flow\"\ndupin_e2e_future_setting = true\n" | save --force ($config | path join dupin config.toml)
    try { prepare-scripts $out } catch {|failure|
        try { cleanup-scripts $out }
        try { cleanup $out }
        error make $failure
    }
    let log = $out | path join dupin.log
    let data = $out | path join data
    rm --recursive --force $data
    mkdir $data
    let runtime = $out | path join runtime
    mkdir $runtime
    ^chmod 700 $runtime
    let binary = $app | path join Contents MacOS dupin
    let dupin = job spawn {
        with-env {
            XDG_CONFIG_HOME: $config
            XDG_DATA_HOME: $data
            XDG_RUNTIME_DIR: $runtime
            DUPIN_TRACE: '1'
            DUPIN_E2E_SCRIPT_OUT: ($out | path join script-fixtures)
        } {
            ^($app | path join Contents MacOS dupin) o+e> $log
        }
    }

    let failure = try {

        # Dupin is running well before its on_ready callback registers the hotkey.
        wait-for-log $log 'dupin-trace: hotkey alt+space registered'
        evidence $out launched
        let run = {
            (do
                $suite
                $out
                {binary: $binary, runtime: $runtime, data: $data}
            )
        }
        if $record {
            record $out $run
        } else {
            do $run
        }
        null
    } catch {|error| $error }

    let stop_failure = try {
        job kill $dupin
        null
    } catch {|error|
        $error
    }
    let script_cleanup_failure = try {
        cleanup-scripts $out
        null
    } catch {|error|
        $error
    }
    let cleanup_failure = try {
        cleanup $out
        null
    } catch {|error|
        $error
    }
    if $failure != null { error make $failure }
    if $stop_failure != null { error make $stop_failure }
    if $script_cleanup_failure != null { error make $script_cleanup_failure }
    if $cleanup_failure != null { error make $cleanup_failure }
    # A shader that fails falls back to the built-in one with a warning, which the
    # screenshots alone wouldn't show.
    if (open --raw $log | lines | any {|line| $line =~ '^dupin: backdrop (shader|pipeline)' }) {
        error make {msg: $"the backdrop shader failed; see ($log)"}
    }
}

export def wait-condition [name: string, ready: closure] {
    for _ in 1..20 {
        if (do $ready) { return }
        sleep 1sec
    }
    error make {msg: $'Timed out waiting for ($name)'}
}

export def file-proof [file: path]: nothing -> record {
    if ($file | path exists) {
        {
            exists: true
            sha256: (open --raw $file | hash sha256)
        }
    } else {
        {exists: false, sha256: null}
    }
}

# Each file below `dir`, by its path relative to it, with its digest, so any write shows.
export def dir-proof [dir: path]: nothing -> list<record> {
    if not ($dir | path exists) { return [] }
    glob ($dir | path join '**' '*') --no-dir
    | sort
    | each {|file| {path: ($file | path relative-to $dir), sha256: (open --raw $file | hash sha256)} }
}

# Fails when any file below `dir` holds `text` as it was typed.
export def assert-sealed [dir: path, text: string] {
    let clear = glob ($dir | path join '**' '*') --no-dir
    | where {|file| open --raw $file | into binary | bytes index-of ($text | into binary) | $in >= 0 }
    if ($clear | is-not-empty) {
        error make {msg: $'($text) is on disk in the clear: ($clear | str join ", ")'}
    }
}

export def log-count [log: path, text: string]: nothing -> int {
    open --raw $log | lines | where {|line| $line | str contains $text } | length
}

# Run agent-desktop, keep its reply in <out>/<name>.json, and return it parsed.
export def --wrapped ad [out: path, name: string, ...args: string]: nothing -> record {
    let result = ^agent-desktop ...$args | complete
    $result.stdout | save --force ($out | path join $'($name).json')
    $result.stderr | save --append ($out | path join agent-desktop.stderr)
    try {
        $result.stdout | from json
    } catch {
        {ok: false}
    }
}

# Press chords such as `cmd+shift+s`, `alt+space` or `down` in turn through System Events, keeping
# its reply in <out>/<name>.json. `agent-desktop press` posts a lone key event carrying the modifier flags from
# a process that exits straight after, and on the runner such a press sometimes never reached
# Dupin: not as a key, and not as the hotkey.
export def press [out: path, name: string, ...chords: string]: nothing -> record {
    let result = ^osascript -e (key-script ...$chords) | complete
    system-events-reply $out $name {chords: $chords} $result
}

# Type `text` one character at a time through System Events, as `press` does, keeping its reply
# in <out>/<name>.json. Characters go through the keyboard layout, as a person's typing does.
export def type-text [out: path, name: string, text: string]: nothing -> record {
    let result = (
        ^osascript -e 'on run argv
            tell application "System Events"
                repeat with typedKey in characters of (item 1 of argv)
                    keystroke (typedKey as text)
                    delay 0.12
                end repeat
            end tell
        end run' $text
        | complete
    )
    system-events-reply $out $name {text: $text} $result
}

def system-events-reply [
    out: path
    name: string
    sent: record
    result: record
]: nothing -> record {
    let reply = {
        ok: ($result.exit_code == 0)
        ...$sent
        stderr: $result.stderr
    }
    $reply | to json | save --force ($out | path join $'($name).json')
    if not $reply.ok {
        error make {msg: $'System Events could not send ($name): ($result.stderr)'}
    }
    $reply
}

# Retry an agent-desktop command once a second until `done` accepts its reply.
export def --wrapped poll [
    out: path
    name: string
    done: closure
    --tries: int = 10
    ...args: string
] {
    mut reply = {ok: false}
    for _ in (attempts $tries) {
        $reply = ad $out $name ...$args
        if (do $done $reply) { return $reply }
        sleep 1sec
    }
    error make {msg: $"timed out waiting for ($name): ($reply | to json --raw)"}
}

export def wait-for-log [
    log: path
    line: string
    --tries: int = 30
    --after: int = 0
] {
    for _ in (attempts $tries) {
        if ($log | path exists) and ((log-count $log $line) > $after) { return }
        sleep 1sec
    }
    error make {msg: $"timed out waiting for ($line) in ($log)"}
}

export def mentions [reply: record, name: string]: nothing -> bool {
    $reply.data? | to json --raw | str contains $'"($name)"'
}

export def evidence [out: path, name: string, --app: string = $DUPIN] {
    ad $out $'($name)-snapshot' snapshot --app $app | ignore
    let shot = ^screencapture -x ($out | path join $'($name).png') | complete
    if $shot.exit_code != 0 {
        error make {msg: $"screencapture failed for ($name): ($shot.stderr)"}
    }
}

# Wait until `app` is the frontmost application, so a screenshot shows its window.
export def wait-frontmost [app: string, --tries: int = 20] {
    let script = 'tell application "System Events" to get name of first application process whose frontmost is true'
    mut front = ''
    for _ in (attempts $tries) {
        let reply = (^osascript -e $script | complete)
        $front = ($reply.stdout | str trim)
        if (
            $reply.exit_code == 0
            and ($front | str lowercase) == ($app | str lowercase)
        ) { return }
        sleep 500ms
    }
    error make {msg: $"timed out waiting for ($app) to be frontmost; ($front) is"}
}

def attempts [tries: int]: nothing -> range {
    if $tries < 1 { error make {msg: '--tries must be at least 1'} }
    1..$tries
}
