use driver.nu [
    ad
    log-count
    poll
    snap
    wait-condition
    wait-for-log
]
use ../e2e-script-fixtures.nu script-processes-stopped
use ../e2e-state.nu [has-match is-focused-query]

def type-script-text [value: string] {
    ^osascript -e 'on run argv
        tell application "System Events"
            repeat with typedKey in characters of (item 1 of argv)
                keystroke (typedKey as text)
                delay 0.12
            end repeat
        end tell
    end run' $value
}

def script-query [out: path, title: string] {
    ad $out $'($title)-summon' press alt+space | ignore
    (poll
        $out
        $'($title)-empty'
        {|r| is-focused-query $r '' }
        find
        --app
        Dupin
        --role
        textfield
        --name
        'Search apps'
        --exact
        --first
    ) | ignore
    let stale = (ad
        $out
        $'($title)-stale-card'
        find
        --app
        Dupin
        --name
        '↵ Copy · ⌘O Full output'
        --exact
        --first
    )
    if $stale.data?.match? != null { error make {msg: 'Launcher reopened with an old script card'} }
    type-script-text $title
    (poll
        $out
        $'($title)-query'
        {|r| is-focused-query $r $title }
        find
        --app
        Dupin
        --role
        textfield
        --name
        'Search apps'
        --exact
        --first
    ) | ignore
    (poll
        $out
        $'($title)-result'
        {|r| has-match $r }
        find
        --app
        Dupin
        --name
        $title
        --exact
        --first
    ) | ignore
}

export def scripts-stage [out: path] {
    let log = $out | path join dupin.log
    let scripts = $out | path join config dupin scripts
    let extra = $scripts | path join refresh-fixture
    let echo_searched = 'dupin-trace: results done query="E2E script echo"'
    let focused = {|r| $r.ok and (
        $r.data?
        | default []
        | any {|w| $w.title == 'Dupin Script Output' and $w.is_focused }
    ) }
    let found = {|r| has-match $r }
    try {
        (poll
            $out
            scripts-panel-hidden
            {|r| $r.ok and not ($r.data? | default [] | any {|w| $w.is_focused }) }
            list-windows
            --app
            Dupin
        ) | ignore
        script-query $out 'E2E script echo'
        ad $out scripts-arguments press tab | ignore
        (poll
            $out
            scripts-argument-field
            {|r| is-focused-query $r '' }
            find
            --app
            Dupin
            --role
            textfield
            --name
            'Script arguments'
            --exact
            --first
        ) | ignore
        let arguments = "first '$HOME' '*.rs'"
        type-script-text $arguments
        wait-for-log $log $echo_searched
        let before_argument_refresh = log-count $log $echo_searched
        cp ($scripts | path join failure) $extra
        ^chmod 700 $extra
        wait-for-log --after $before_argument_refresh $log $echo_searched
        (poll
            $out
            scripts-arguments-value
            {|r| is-focused-query $r $arguments }
            find
            --app
            Dupin
            --role
            textfield
            --name
            'Script arguments'
            --exact
            --first
        ) | ignore
        snap $out scripts-arguments
        ad $out scripts-submit press return | ignore
        (poll
            $out
            scripts-inline
            $found
            find
            --app
            Dupin
            --name
            'first | $HOME | *.rs'
            --exact
            --first
        ) | ignore
        let argv = open ($out | path join script-fixtures argv.json)
        if $argv != [first '$HOME' '*.rs'] { error make {msg: 'Script arguments were expanded or changed'} }
        let before_card_refresh = log-count $log $echo_searched
        rm $extra
        wait-for-log --after $before_card_refresh $log $echo_searched
        (poll
            $out
            scripts-inline-finished
            {|r| $r.ok and (($r.data?.match?.name? | default '') =~ '^Finished · [0-9]+\.[0-9] s$') }
            find
            --app
            Dupin
            --role
            statictext
            --name
            'Finished ·'
            --first
        ) | ignore
        snap $out scripts-inline
        let closed = log-count $log 'dupin-trace: panel closed '
        'Dupin E2E clipboard baseline' | ^pbcopy
        ad $out scripts-inline-copy press return | ignore
        wait-for-log --after $closed $log 'dupin-trace: panel closed '
        let copied = ^pbpaste
        $copied | save ($out | path join scripts-inline-copied.txt)
        if $copied != 'first | $HOME | *.rs' { error make {msg: 'Inline Enter did not copy the script result'} }

        script-query $out 'E2E script stream'
        ad $out scripts-stream-start press return | ignore
        (poll
            $out
            scripts-output-window
            $focused
            list-windows
            --app
            Dupin
        ) | ignore
        (poll
            $out
            scripts-stream-line
            $found
            find
            --app
            Dupin
            --name
            'Stream line 3'
            --exact
            --first
        ) | ignore
        (poll
            $out
            scripts-still-running
            $found
            find
            --app
            Dupin
            --name
            Running
            --exact
            --first
        ) | ignore
        snap $out scripts-streaming
        ad $out scripts-cancel press cmd+. | ignore
        (poll
            $out
            scripts-cancelled
            $found
            find
            --app
            Dupin
            --name
            Cancelled
            --exact
            --first
        ) | ignore
        wait-condition 'script fixture process group exit' { script-processes-stopped $out }
        snap $out scripts-cancelled
        let echo = (poll
            $out
            scripts-echo-in-history
            {|r| $r.ok and $r.data?.match?.ref_id? != null }
            find --app Dupin --role button --name 'E2E script echo Finished' --exact --first
        )
        ad $out scripts-select-echo click $echo.data.match.ref_id | ignore
        ad $out scripts-repeat-echo press cmd+r | ignore
        (poll
            $out
            scripts-repeated-echo-selected
            $found
            find --app Dupin --name 'Repeated echo' --exact --first
        ) | ignore
        ad $out scripts-output-close press cmd+w | ignore
        (poll
            $out
            scripts-output-closed
            {|r| $r.ok and not ($r.data? | default [] | any {|w| $w.title == 'Dupin Script Output' }) }
            list-windows
            --app
            Dupin
        ) | ignore

        script-query $out 'E2E script failure'
        ad $out scripts-failure-start press return | ignore
        (poll
            $out
            scripts-failure-window
            $focused
            list-windows
            --app
            Dupin
        ) | ignore
        (poll
            $out
            scripts-failure-status
            $found
            find
            --app
            Dupin
            --name
            'Exited with code 3'
            --exact
            --first
        ) | ignore
        (poll
            $out
            scripts-failure-stderr
            $found
            find
            --app
            Dupin
            --name
            'stderr: E2E script failed deliberately'
            --exact
            --first
        ) | ignore
        snap $out scripts-failure
        ad $out scripts-failure-close press cmd+w | ignore
    } catch {|failure|
        try { snap $out scripts-failure-evidence }
        error make $failure
    }
}
