#!/usr/bin/env nix
#! nix shell --inputs-from .. nixpkgs#nushell --command nu
# Drive a built Dupin.app through its accessibility tree with agent-desktop:
# hotkey → panel → type "calculator" → ↩ → Calculator is running, then Settings' pages. The
# panel draws the built-in backdrop shader, so the screenshots show it.
# Snapshots hold whatever the panel lists, so run this only on a disposable machine.

use e2e-recording.nu record
use e2e-state.nu is-focused-query
use e2e-file-fixtures.nu [prepare cleanup viewer-running readable-trash]

# Run the scenario against `app`, keeping each step's last agent-desktop reply, a snapshot and a
# screenshot per stage, and Dupin's log in `out`.
def "main run" [app: path, --out: path = 'target/e2e', --record] {
    mkdir $out
    let out = $out | path expand
    let app = $app | path expand

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

    if (viewer-running) {
        error make {msg: 'Cannot establish ownership of the file-action viewer'}
    }
    readable-trash
    if ($out | path join fixtures | path exists) {
        error make {msg: 'File-action fixtures already exist'}
    }
    if (glob ($env.HOME | path join '.Trash/trash-me*.dupine2e') | is-not-empty) {
        error make {msg: 'File-action Trash fixtures already exist'}
    }
    try { prepare $out } catch {|failure|
        try { cleanup $out }
        error make $failure
    }

    # Spotlight holds ⌘Space on a fresh macOS.
    let config = $out | path join config
    mkdir ($config | path join dupin)
    "hotkey = \"alt+space\"\nbackdrop = \"flow\"\n" | save --force ($config | path join dupin config.toml)
    let log = $out | path join dupin.log
    let data = $out | path join data
    rm --recursive --force $data
    mkdir $data
    let dupin = job spawn {
        with-env {XDG_CONFIG_HOME: $config, XDG_DATA_HOME: $data, DUPIN_TRACE: '1'} {
            ^($app | path join Contents MacOS dupin) o+e> $log
        }
    }

    let failure = try {

        # Dupin is running well before its on_ready callback registers the hotkey.
        wait-for-log $log 'dupin-trace: hotkey alt+space registered'
        snap $out launched
        if $record { record $out { scenario $out } } else { scenario $out }
        null
    } catch {|error| $error }

    let stop_failure = try {
        job kill $dupin
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
    if $cleanup_failure != null { error make $cleanup_failure }
    # A shader that fails falls back to the built-in one with a warning, which the
    # screenshots alone wouldn't show.
    if (open --raw $log | lines | any {|line| $line =~ '^dupin: backdrop (shader|pipeline)' }) {
        error make {msg: $"the backdrop shader failed; see ($log)"}
    }
    print 'e2e: Quick Look restored the query; Calculator, Settings, Bluetooth, Open with… and confirmed Trash passed'
}

def scenario [out: path] {
    let log = $out | path join dupin.log
    close-startup-welcome $out
    ad $out press-hotkey press alt+space | ignore
    let focused = {|r| $r.ok and 'focused' in ($r.data?.match?.states? | default []) }
    poll $out field $focused find --app Dupin --role textfield --first
    # Past the backdrop's fade-in, so the screenshot shows the shader.
    sleep 500ms
    snap $out summoned
    let found = {|r| $r.ok and $r.data?.match? != null }
    snapshot-stage $out $found

    # `type` needs a settable AX value or the main window, and the field has neither: the
    # panel never activates, so it is key but not main. Press keys as a person would, from one
    # process: one `agent-desktop press` per key lost keys before they reached Dupin.
    ^osascript -e 'tell application "System Events"
        repeat with typedKey in characters of "calculator"
            keystroke (typedKey as text)
            delay 0.12
        end repeat
    end tell'
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

    ad $out preview-shortcut press cmd+y | ignore
    # Quick Look is visible in recordings but absent from Dupin's AX window list.
    wait-for-log $log 'dupin-trace: quick-look shown visible=true key=true'
    snap $out quick-look
    let left_before = open --raw $log | lines | where {|line| $line | str contains 'dupin-trace: key Left ' } | length
    let closed_before = open --raw $log | lines | where {|line| $line | str contains 'dupin-trace: quick-look picker input visible=false key=false' } | length
    ad $out close-preview press escape | ignore
    let restored = {|r| is-focused-query $r calculator }
    (poll
        $out
        quick-look-restored
        $restored
        find
        --app
        Dupin
        --role
        textfield
        --first
    )
    ^osascript -e 'tell application "System Events" to key code 123'
    wait-for-log --after $left_before $log 'dupin-trace: key Left '
    wait-for-log --after $closed_before $log 'dupin-trace: quick-look picker input visible=false key=false'
    ^osascript -e 'tell application "System Events" to key code 124'
    snap $out quick-look-restored

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
    # Accessory windows can receive keys while System Events names another frontmost app.
    let settings = {|r| $r.ok and (
        $r.data
        | any {|window| $window.title == 'Dupin Settings' and $window.is_focused }
    ) }
    poll $out settings-window $settings list-windows --app Dupin
    snap $out settings-created
    let before_key = (ad
        $out
        settings-before-key
        find
        --app
        Dupin
        --role
        group
        --name
        General
        --exact
        --first
    )
    if not $before_key.ok or $before_key.data?.match? == null or 'focused' in ($before_key.data?.match?.states? | default []) {
        error make {msg: 'General must exist without keyboard focus before the Tab test'}
    }
    ad $out settings-tab press tab | ignore
    (poll
        $out
        settings-keyboard-focused
        $focused
        find
        --app
        Dupin
        --role
        group
        --name
        General
        --exact
        --first
    )
    snap $out settings-general
    # Each page in the sidebar in turn: a ref from before a click goes stale once the page
    # changes, so find the next one afresh.
    let search = poll $out search $page find --app Dupin --name Search --exact --first
    ad $out open-search click $search.data.match.ref_id | ignore
    (poll
        $out
        search-page
        $found
        find
        --app
        Dupin
        --name
        'Files shown'
        --exact
        --first
    )
    snap $out settings-search
    let appearance = (poll
        $out
        appearance
        $page
        find
        --app
        Dupin
        --name
        Appearance
        --exact
        --first
    )
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
    poll $out settings-window-again $settings list-windows --app Dupin
    snap $out settings-appearance

    ad $out close-settings press cmd+w | ignore
    ad $out settings-search-hotkey press alt+space | ignore
    (poll
        $out
        settings-search-field
        $focused
        find
        --app
        Dupin
        --role
        textfield
        --first
    )
    ^osascript -e 'tell application "System Events"
        repeat with typedKey in characters of "dupin settings"
            keystroke (typedKey as text)
            delay 0.12
        end repeat
    end tell'
    (poll
        $out
        settings-search-query
        {|r| $r.ok and $r.data?.match?.value? == 'dupin settings' }
        find
        --app
        Dupin
        --role
        textfield
        --first
    )
    (poll
        $out
        settings-search-result
        $found
        find
        --app
        Dupin
        --name
        'Dupin Settings Preferences'
        --exact
        --first
    )
    let log = $out | path join dupin.log
    let requests_before = open --raw $log | lines | where {|line| $line | str contains 'dupin-trace: settings requested ' } | length
    let closed_before = open --raw $log | lines | where {|line| $line | str contains 'dupin-trace: settings panel closed ' } | length
    ad $out open-settings-result press return | ignore
    wait-for-log --after $requests_before $log 'dupin-trace: settings requested '
    wait-for-log --after $closed_before $log 'dupin-trace: settings panel closed '
    poll $out settings-from-result $settings list-windows --app Dupin
    snap $out settings-from-result

    system-pane $out $focused $found
    file-actions-stage $out $found
}

def close-startup-welcome [out: path] {
    let settings_closed = {|r| $r.ok and not ($r.data | any {|w| $w.title == 'Dupin Settings' }) }
    let windows = ad $out startup-windows list-windows --app Dupin
    if not $windows.ok { error make {msg: 'Could not inspect Dupin startup windows'} }
    if (do $settings_closed $windows) { return }

    let close = (poll
        $out
        welcome-close-button
        {|r| $r.ok and $r.data?.match?.ref_id? != null }
        find --app Dupin --role button --name Close --exact --first
    )
    let clicked = ad $out welcome-close click $close.data.match.ref_id
    if not $clicked.ok { error make {msg: 'Could not close the startup welcome'} }
    poll $out welcome-closed $settings_closed list-windows --app Dupin | ignore
}

def file-actions-stage [out: path, found: closure] {
    let log = $out | path join dupin.log
    let root = $out | path join fixtures
    let open_path = $root | path join open-me.dupine2e
    let trash_path = $root | path join trash-me.dupine2e
    try {
        file-search $out open-me $found
        let loading = count-log $log 'dupin-trace: action-screen applications loading'
        let ready = count-log $log 'dupin-trace: action-screen applications ready'
        ad $out file-actions-choose-down press down | ignore
        ad $out file-actions-choose press return | ignore
        wait-for-log --after $loading $log 'dupin-trace: action-screen applications loading'
        wait-for-log --after $ready $log 'dupin-trace: action-screen applications ready'
        let viewer = (poll
            $out
            file-actions-viewer
            $found
            find
            --app
            Dupin
            --name
            'DupinE2EViewer Default'
            --exact
            --first
        )
        snap $out file-actions-chooser
        ad $out file-actions-open click $viewer.data.match.ref_id | ignore
        let marker = $root | path join opened.json
        wait-condition 'fixture application opening the exact path' {
            ($marker | path exists) and (open $marker) == [$open_path]
        }
        snap $out file-actions-opened
        ^osascript -e 'tell application id "org.dupin.e2e.viewer" to quit'
        wait-condition 'fixture viewer quitting' { not (viewer-running) }

        file-search $out trash-me $found
        ^osascript -e 'tell application "System Events"
            repeat 8 times
                key code 125
            end repeat
        end tell'
        let confirm = count-log $log 'dupin-trace: action-screen confirm'
        ad $out file-actions-trash-review press return | ignore
        wait-for-log --after $confirm $log 'dupin-trace: action-screen confirm'
        (poll
            $out
            file-actions-trash-question
            $found
            find
            --app
            Dupin
            --name
            'Move “trash-me.dupine2e” to the Trash?'
            --exact
            --first
        ) | ignore
        (poll
            $out
            file-actions-trash-path
            $found
            find
            --app
            Dupin
            --name
            $trash_path
            --exact
            --first
        ) | ignore
        snap $out file-actions-trash-confirm
        let list = count-log $log 'dupin-trace: action-screen list'
        ad $out file-actions-trash-cancel press return | ignore
        wait-for-log --after $list $log 'dupin-trace: action-screen list'
        let confirm = count-log $log 'dupin-trace: action-screen confirm'
        ad $out file-actions-trash-review-again press return | ignore
        wait-for-log --after $confirm $log 'dupin-trace: action-screen confirm'
        if not ($trash_path | path exists) { error make {msg: 'Enter on Cancel removed the fixture'} }
        snap $out file-actions-trash-cancelled
        ad $out file-actions-trash-select press down | ignore
        ad $out file-actions-trash-confirm press return | ignore
        wait-condition 'fixture moving to the Trash' {
            not ($trash_path | path exists) and (glob ($env.HOME | path join '.Trash/trash-me*.dupine2e') | length) == 1
        }
        let moved = glob ($env.HOME | path join '.Trash/trash-me*.dupine2e') | first
        if (open --raw $moved | hash sha256) != (open --raw ($root | path join trash.sha256)) {
            error make {msg: 'The Trash item is not the owned fixture'}
        }
        cp $moved ($root | path join moved-trash-me.dupine2e)
        let row_name = file-row-name $out trash-me
        (poll
            $out
            file-actions-row-removed
            {|r| $r.ok and not (mentions $r $row_name) }
            snapshot
            --app
            Dupin
        ) | ignore
        (poll
            $out
            file-actions-trash-notice
            $found
            find
            --app
            Dupin
            --name
            'Moved “trash-me.dupine2e” to the Trash'
            --exact
            --first
        ) | ignore
        snap $out file-actions-trash-finished
        ad $out file-actions-close press escape | ignore
    } catch {|failure|
        try { snap $out file-actions-failure }
        try { cp $log ($out | path join file-actions-failure.log) }
        error make $failure
    }
}

def file-row-name [out: path, name: string]: nothing -> string {
    $'($name).dupine2e (file-parent $out)'
}

def file-parent [out: path]: nothing -> string {
    let parent = $out | path join fixtures
    let parent = if ($parent | str starts-with $env.HOME) {
        $parent | str replace $env.HOME '~'
    } else {
        $parent
    }
    $parent
}

def file-search [out: path, query: string, found: closure] {
    let file = $'($query).dupine2e'
    let log = $out | path join dupin.log
    (poll
        $out
        $'file-actions-($query)-apps'
        {|r| $r.ok and not (mentions $r 'System Settings') }
        list-apps
    ) | ignore
    ad $out $'file-actions-($query)-summon' press alt+space | ignore
    (poll
        $out
        $'file-actions-($query)-panel'
        {|r| is-focused-query $r '' }
        find
        --app
        Dupin
        --role
        textfield
        --first
    ) | ignore
    ^osascript -e 'tell application "System Events" to keystroke " "'
    (poll
        $out
        $'file-actions-($query)-mode'
        {|r| is-focused-query $r '' }
        find
        --app
        Dupin
        --role
        textfield
        --name
        'Search files'
        --exact
        --first
    ) | ignore
    ^osascript -e 'on run argv
        tell application "System Events"
            repeat with typedKey in characters of (item 1 of argv)
                keystroke (typedKey as text)
                delay 0.12
            end repeat
        end tell
    end run' $file
    (poll
        $out
        $'file-actions-($query)-query'
        {|r| is-focused-query $r $file }
        find
        --app
        Dupin
        --role
        textfield
        --name
        'Search files'
        --exact
        --first
    ) | ignore
    (poll
        $out
        $'file-actions-($query)-row'
        $found
        find
        --app
        Dupin
        --name
        (file-row-name $out $query)
        --exact
        --first
    ) | ignore
    let right = count-log $log 'dupin-trace: key Right '
    ad $out $'file-actions-($query)-actions' press right | ignore
    wait-for-log --after $right $log 'dupin-trace: key Right '
    (poll
        $out
        $'file-actions-($query)-context-title'
        $found
        find
        --app
        Dupin
        --name
        $file
        --exact
        --first
    ) | ignore
    let parent = file-parent $out
    (poll
        $out
        $'file-actions-($query)-context-path'
        $found
        find
        --app
        Dupin
        --name
        $parent
        --exact
        --first
    ) | ignore
}

def count-log [log: path, text: string]: nothing -> int {
    open --raw $log | lines | where {|line| $line | str contains $text } | length
}

def wait-condition [name: string, ready: closure] {
    for _ in 1..20 {
        if (do $ready) { return }
        sleep 1sec
    }
    error make {msg: $'Timed out waiting for ($name)'}
}

def system-pane [out: path, focused: closure, found: closure] {
    let before = poll $out bluetooth-evidence-before-apps {|r| $r.ok } list-apps
    if (mentions $before 'System Settings') {
        error make {
            msg: 'System Settings was already running; a stale pane could falsely pass'
        }
    }
    ad $out bluetooth-evidence-close-preferences press cmd+w | ignore
    ad $out bluetooth-evidence-hotkey press alt+space | ignore
    (poll
        $out
        bluetooth-evidence-field
        $focused
        find
        --app
        Dupin
        --role
        textfield
        --first
    )
    ^osascript -e 'tell application "System Events"
        repeat with typedKey in characters of "bluetooth"
            keystroke (typedKey as text)
            delay 0.12
        end repeat
    end tell'
    (poll
        $out bluetooth-evidence-query
        {|r| $r.ok and $r.data?.match?.value? == 'bluetooth' }
        find --app Dupin --role textfield --first
    )
    let result = (poll
        $out bluetooth-evidence-selected-result $found
        find --app Dupin --name 'Bluetooth System Settings' --exact --first
    )
    if $result.data?.match?.ref_id? == null {
        error make {msg: 'Bluetooth result has no actionable accessibility reference'}
    }
    snap $out bluetooth-evidence-before
    let log = $out | path join dupin.log
    cp $log ($out | path join bluetooth-evidence-before-trace.log)
    let failure = try {
        let activated = (ad
            $out
            bluetooth-evidence-activate-result
            click
            $result.data.match.ref_id
        )
        if not $activated.ok {
            error make {msg: 'Could not activate the exact Bluetooth launcher result'}
        }
        wait-frontmost 'System Settings'
        let destination = {|r| $r.ok and (
            $r.data | any {|window| $window.title == 'Bluetooth' and $window.is_focused }
        ) }
        (poll
            $out
            bluetooth-evidence-destination
            $destination
            list-windows
            --app
            'System Settings'
        )
        snap --app 'System Settings' $out bluetooth-evidence-after
        null
    } catch {|error|
        try { snap --app 'System Settings' $out bluetooth-evidence-failure }
        $error
    }
    let trace_failure = try {
        cp $log ($out | path join bluetooth-evidence-after-trace.log)
        null
    } catch {|error| $error }
    # The absence check above establishes ownership of this CI-launched app.
    let cleanup_failure = try {
        ^osascript -e 'tell application "System Settings" to quit'
        null
    } catch {|error| $error }
    if $failure != null { error make $failure }
    if $trace_failure != null { error make $trace_failure }
    if $cleanup_failure != null { error make $cleanup_failure }
}

def snapshot-stage [out: path, found: closure] {
    let log = $out | path join dupin.log
    let archive = $out | path join data dupin snapshots archive.json
    let history = $env.HOME | path join 'Library/Application Support/Dupin/history.json'
    let before = file-proof $history
    $before | to json | save --force ($out | path join snapshots-history-before.json)
    if ($archive | path exists) { error make {msg: 'Snapshot archive was not isolated'} }
    try {
        let done = log-count $log 'dupin-trace: results done query="calculator"'
        ^osascript -e 'tell application "System Events"
            repeat with typedKey in characters of "calculator"
                keystroke (typedKey as text)
                delay 0.12
            end repeat
        end tell'
        (poll
            $out
            snapshots-query
            {|r| is-focused-query $r calculator }
            find
            --app
            Dupin
            --role
            textfield
            --first
        ) | ignore
        wait-for-log --after $done $log 'dupin-trace: results done query="calculator"'
        (poll
            $out
            snapshots-result
            $found
            find
            --app
            Dupin
            --name
            'Calculator /System/Applications'
            --exact
            --first
        ) | ignore
        ad $out snapshots-save press cmd+shift+s | ignore
        wait-for-log $log 'dupin-trace: snapshot capture accepted'
        wait-for-log $log 'dupin-trace: snapshot saved'
        let saved = open $archive
        if ($saved.entries | length) != 1 or $saved.entries.0.mode != 'apps.search' or $saved.entries.0.query != calculator {
            error make {msg: 'Snapshot did not capture the settled Apps query'}
        }
        let archive_before = file-proof $archive
        cp $archive ($out | path join snapshots-archive-after-save.json)
        snap $out snapshots-saved

        let closed = log-count $log 'dupin-trace: panel closed '
        ad $out snapshots-open press cmd+shift+o | ignore
        wait-for-log --after $closed $log 'dupin-trace: panel closed '
        poll $out snapshots-window {|r| $r.ok and (
            $r.data?
            | default []
            | any {|w| $w.title == 'Dupin Snapshots' and $w.is_focused }
        ) } list-windows --app Dupin | ignore
        (poll
            $out
            snapshots-field
            {|r| is-focused-query $r '' }
            find
            --app
            Dupin
            --role
            textfield
            --name
            'Search saved snapshots'
            --exact
            --first
        ) | ignore
        (poll
            $out
            snapshots-detail
            $found
            find
            --app
            Dupin
            --name
            'Historical snapshot · saved content'
            --exact
            --first
        ) | ignore
        snap $out snapshots-opened

        ^osascript -e 'tell application "System Events" to keystroke "zzz"'
        (poll
            $out
            snapshots-filter
            {|r| is-focused-query $r zzz }
            find
            --app
            Dupin
            --role
            textfield
            --name
            'Search saved snapshots'
            --exact
            --first
        ) | ignore
        (poll
            $out
            snapshots-no-matches
            $found
            find
            --app
            Dupin
            --name
            'No matches for this search.'
            --exact
            --first
        ) | ignore
        snap $out snapshots-filtered
        ^osascript -e 'tell application "System Events" to key code 51 using command down'
        (poll
            $out
            snapshots-cleared
            {|r| is-focused-query $r '' }
            find
            --app
            Dupin
            --role
            textfield
            --name
            'Search saved snapshots'
            --exact
            --first
        ) | ignore
        (poll
            $out
            snapshots-restored
            $found
            find
            --app
            Dupin
            --name
            'Historical snapshot · saved content'
            --exact
            --first
        ) | ignore
        if (file-proof $archive) != $archive_before { error make {msg: 'Editing snapshot search changed the archive'} }
        snap $out snapshots-search-cleared

        let delete = (poll
            $out
            snapshots-delete
            $found
            find
            --app
            Dupin
            --role
            button
            --name
            'Delete…'
            --exact
            --first
        )
        ad $out snapshots-delete-click click $delete.data.match.ref_id | ignore
        let prompt = 'Delete this saved snapshot? This cannot be undone.'
        (poll
            $out
            snapshots-confirmation
            $found
            find
            --app
            Dupin
            --name
            $prompt
            --exact
            --first
        ) | ignore
        snap $out snapshots-confirm
        if not (open --raw ($out | path join snapshots-confirm-snapshot.json) | str contains $prompt) {
            error make {msg: 'Snapshot tree omitted the visible delete confirmation'}
        }
        let cancel = (poll
            $out
            snapshots-cancel
            $found
            find
            --app
            Dupin
            --role
            button
            --name
            Cancel
            --exact
            --first
        )
        ad $out snapshots-cancel-click click $cancel.data.match.ref_id | ignore
        (poll
            $out
            snapshots-cancelled
            {|r| $r.ok and not ($r | to json --raw | str contains $prompt) }
            snapshot
            --app
            Dupin
        ) | ignore
        (poll
            $out
            snapshots-delete-restored
            $found
            find
            --app
            Dupin
            --role
            button
            --name
            'Delete…'
            --exact
            --first
        ) | ignore
        if (file-proof $archive) != $archive_before { error make {msg: 'Cancelling snapshot deletion changed the archive'} }
        cp $archive ($out | path join snapshots-archive-after-cancel.json)
        for event in ['capture accepted' saved] {
            if (log-count $log $'dupin-trace: snapshot ($event)') != 1 {
                error make {msg: $'Expected exactly one snapshot ($event)'}
            }
        }
        for event in ['capture refused' deleted 'write failed'] {
            if (log-count $log $'dupin-trace: snapshot ($event)') != 0 {
                error make {msg: $'Unexpected snapshot ($event)'}
            }
        }
        ad $out snapshots-close press cmd+w | ignore
        (poll
            $out
            snapshots-closed
            {|r| $r.ok and not ($r.data? | default [] | any {|w| $w.title == 'Dupin Snapshots' }) }
            list-windows
            --app
            Dupin
        ) | ignore
        ad $out snapshots-summon press alt+space | ignore
        (poll
            $out
            snapshots-panel
            {|r| is-focused-query $r '' }
            find
            --app
            Dupin
            --role
            textfield
            --first
        ) | ignore
        snap $out snapshots-resummoned
        let after = file-proof $history
        $after | to json | save --force ($out | path join snapshots-history-after.json)
        if $after != $before { error make {msg: 'Snapshot flow changed launch history'} }
    } catch {|failure|
        try { snap $out snapshots-failure }
        try { cp $log ($out | path join snapshots-failure.log) }
        error make $failure
    }
}

def file-proof [file: path]: nothing -> record {
    if ($file | path exists) {
        {
            exists: true
            sha256: (open --raw $file | hash sha256)
        }
    } else {
        {exists: false, sha256: null}
    }
}

def log-count [log: path, text: string]: nothing -> int {
    open --raw $log | lines | where {|line| $line | str contains $text } | length
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

def wait-for-log [
    log: path
    line: string
    --tries: int = 30
    --after: int = 0
] {
    for attempt in 1..$tries {
        if ($log | path exists) and ((open --raw $log | lines | where {|entry| $entry | str contains $line } | length) > $after) { return }
        sleep 1sec
    }
    error make {msg: $"timed out waiting for ($line) in ($log)"}
}

def mentions [reply: record, name: string]: nothing -> bool {
    $reply.data? | to json --raw | str contains $'"($name)"'
}

def snap [out: path, name: string, --app: string = 'Dupin'] {
    ad $out $'($name)-snapshot' snapshot --app $app | ignore
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

def main [] {
    print 'usage: scripts/e2e.nu run <Dupin.app> [--out <dir>] [--record]'
}
