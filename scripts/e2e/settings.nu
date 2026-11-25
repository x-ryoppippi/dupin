use driver.nu [
    DUPIN
    ad
    log-count
    poll
    press
    snap
    type-text
    wait-for-log
    wait-frontmost
]
use ../e2e-state.nu [has-match is-focused-match]

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
    snap $out settings-created
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
    snap $out settings-general
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
    snap $out settings-search
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
    snap $out settings-appearance

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
    snap $out settings-from-result
}
