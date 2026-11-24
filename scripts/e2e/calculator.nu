use driver.nu [
    ad
    log-count
    mentions
    poll
    press
    snap
    wait-for-log
]
use ../e2e-state.nu [has-match is-focused-query]

export def calculator-stage [out: path] {
    let log = $out | path join dupin.log
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
    let found = {|r| has-match $r }
    poll $out result $found find --app Dupin --name $row --exact --first
    snap $out results

    press $out preview-shortcut cmd+y | ignore
    # Quick Look is visible in recordings but absent from Dupin's AX window list.
    wait-for-log $log 'dupin-trace: quick-look shown visible=true key=true'
    snap $out quick-look
    let left_before = log-count $log 'dupin-trace: key Left '
    let closed_before = log-count $log 'dupin-trace: quick-look picker input visible=false key=false'
    press $out close-preview escape | ignore
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
    press $out press-return return | ignore
    poll --tries 15 $out opened {|r| $r.ok and (mentions $r Calculator) } list-apps
    snap $out opened
}
