use driver.nu [
    DUPIN
    ad
    log-count
    poll
    press
    evidence
    type-text
    wait-for-log
    wait-frontmost
]
use ../e2e-state.nu [has-match is-focused-match is-focused-query]

export def settings-stage [out: path] {
    let focused = {|r| is-focused-match $r }
    let found = {|r| has-match $r }
    # Calculator can still be coming forward; summoned before then, the panel would lose focus
    # to it and hide before ⌘, arrives.
    wait-frontmost Calculator
    press $out press-hotkey-again alt+space | ignore
    poll $out field-again $focused find --app $DUPIN --role textfield --first
    press $out open-settings 'cmd+,' | ignore
    let page = {|r| $r.ok and $r.data?.match?.ref_id? != null }
    let appearance = (poll
        $out
        settings
        $page
        find
        --app
        $DUPIN
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
    poll $out settings-window $settings list-windows --app $DUPIN
    evidence $out settings-created
    (record-navigation-key
        $out
        'Move to previous result'
        ctrl+k
        navigation_up
    )
    record-navigation-key $out 'Move to next result' ctrl+j navigation_down
    (poll
        $out
        acknowledgements
        $page
        find
        --app
        $DUPIN
        --name
        Acknowledgements
        --exact
        --first
    )
    let before_key = (ad
        $out
        settings-before-key
        find
        --app
        $DUPIN
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
    press $out settings-tab tab | ignore
    (poll
        $out
        settings-keyboard-focused
        $focused
        find
        --app
        $DUPIN
        --role
        group
        --name
        General
        --exact
        --first
    )
    evidence $out settings-general
    # Each page in the sidebar in turn: a ref from before a click goes stale once the page
    # changes, so find the next one afresh.
    let search = poll $out search $page find --app $DUPIN --name Search --exact --first
    ad $out open-search click $search.data.match.ref_id | ignore
    (poll
        $out
        search-page
        $found
        find
        --app
        $DUPIN
        --name
        'Files shown'
        --exact
        --first
    )
    evidence $out settings-search
    let appearance = (poll
        $out
        appearance
        $page
        find
        --app
        $DUPIN
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
        $DUPIN
        --name
        Backdrop
        --exact
        --first
    )
    poll $out settings-window-again $settings list-windows --app $DUPIN
    evidence $out settings-appearance

    press $out close-settings cmd+w | ignore
    press $out settings-search-hotkey alt+space | ignore
    (poll
        $out
        settings-search-field
        $focused
        find
        --app
        $DUPIN
        --role
        textfield
        --first
    )
    type-text $out type-dupin-settings 'dupin settings' | ignore
    (poll
        $out
        settings-search-query
        {|r| $r.ok and $r.data?.match?.value? == 'dupin settings' }
        find
        --app
        $DUPIN
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
        $DUPIN
        --name
        'Dupin Settings Preferences'
        --exact
        --first
    )
    let log = $out | path join dupin.log
    let requests_before = log-count $log 'dupin-trace: settings requested '
    let closed_before = log-count $log 'dupin-trace: settings panel closed '
    press $out open-settings-result return | ignore
    wait-for-log --after $requests_before $log 'dupin-trace: settings requested '
    wait-for-log --after $closed_before $log 'dupin-trace: settings panel closed '
    poll $out settings-from-result $settings list-windows --app $DUPIN
    evidence $out settings-from-result

    # This app uses the disposable XDG profile run-suite created under `out`; recorded keys never
    # touch the machine's ordinary config. Search a common letter and require at least two results
    # before asserting that the custom chord moved the selected row.
    press $out navigation-close-settings cmd+w | ignore
    press $out navigation-summon-panel alt+space | ignore
    (poll
        $out
        navigation-query-field
        $focused
        find
        --app
        $DUPIN
        --role
        textfield
        --first
    ) | ignore
    let log = $out | path join dupin.log
    let results_before = log-count $log 'dupin-trace: results done query="a"'
    type-text $out navigation-type-query a | ignore
    (poll
        $out
        navigation-query
        {|r| is-focused-query $r a }
        find
        --app
        $DUPIN
        --role
        textfield
        --first
    ) | ignore
    wait-for-log --after $results_before $log 'dupin-trace: results done query="a"'
    let result_line = open --raw $log
    | lines
    | where {|line| $line | str contains 'dupin-trace: results done query="a"' }
    | last
    let count = $result_line | parse --regex 'items=(?<items>[0-9]+)' | get items.0 | into int
    if $count < 2 { error make {msg: 'The E2E navigation query needs at least two results'} }
    let moved_before = log-count $log 'dupin-trace: navigation down moved selection 0->1'
    press $out navigation-custom-next ctrl+j | ignore
    wait-for-log --after $moved_before $log 'dupin-trace: navigation down moved selection 0->1'
    let moved_before = log-count $log 'dupin-trace: navigation up moved selection 1->0'
    press $out navigation-custom-previous ctrl+k | ignore
    wait-for-log --after $moved_before $log 'dupin-trace: navigation up moved selection 1->0'
    evidence $out navigation-custom-next
    press $out navigation-close-panel escape | ignore
}

def record-navigation-key [
    out: path
    label: string
    chord: string
    config_key: string
] {
    let button = (poll
        $out
        $'navigation-record-($config_key)'
        {|r| $r.ok and $r.data?.match?.ref_id? != null }
        find
        --app
        $DUPIN
        --role
        button
        --name
        $label
        --exact
        --first
    )
    let clicked = ad $out $'navigation-start-($config_key)' click $button.data.match.ref_id
    if not $clicked.ok { error make {msg: $'Could not start recording ($label)'} }
    press $out $'navigation-press-($config_key)' $chord | ignore
    let path = $out | path join config dupin config.toml
    for _ in 1..20 {
        let value = try {
            open $path | get --optional $config_key
        } catch {
            null
        }
        if $value == $chord { return }
        sleep 250ms
    }
    error make {msg: $'Settings did not save ($config_key) = "($chord)"'}
}
