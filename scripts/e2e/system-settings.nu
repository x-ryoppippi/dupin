use driver.nu [
    ad
    mentions
    poll
    press
    snap
    type-text
    wait-frontmost
]
use ../e2e-state.nu [has-match is-focused-match]

export def system-pane [out: path] {
    let focused = {|r| is-focused-match $r }
    let found = {|r| has-match $r }
    let before = poll $out bluetooth-evidence-before-apps {|r| $r.ok } list-apps
    if (mentions $before 'System Settings') {
        error make {
            msg: 'System Settings was already running; a stale pane could falsely pass'
        }
    }
    press $out bluetooth-evidence-close-preferences cmd+w | ignore
    press $out bluetooth-evidence-hotkey alt+space | ignore
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
    type-text $out bluetooth-evidence-type bluetooth | ignore
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
