use driver.nu [ad poll press snap]
use ../e2e-state.nu is-focused-match

export def launcher-stage [out: path] {
    close-startup-welcome $out
    press $out press-hotkey alt+space | ignore
    let focused = {|r| is-focused-match $r }
    poll $out field $focused find --app Dupin --role textfield --first
    # Past the backdrop's fade-in, so the screenshot shows the shader.
    sleep 500ms
    snap $out summoned
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
