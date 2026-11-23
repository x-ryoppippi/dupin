use driver.nu [
    ad
    log-count
    mentions
    poll
    snap
    wait-condition
    wait-for-log
]
use ../e2e-file-fixtures.nu viewer-running
use ../e2e-state.nu [has-match is-focused-query]

export def file-actions-stage [out: path] {
    let found = {|r| has-match $r }
    let log = $out | path join dupin.log
    let root = $out | path join fixtures
    let open_path = $root | path join open-me.dupine2e
    let trash_path = $root | path join trash-me.dupine2e
    try {
        file-search $out open-me $found
        let loading = log-count $log 'dupin-trace: action-screen applications loading'
        let ready = log-count $log 'dupin-trace: action-screen applications ready'
        click-file-action $out file-actions-choose 'Open with…'
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
        # The accessibility tree only exposes rows near the action list viewport.
        let anchor = (find-file-action
            $out
            file-actions-trash-scroll-anchor
            'Copy to…'
        )
        let scrolled = ad $out file-actions-trash-scroll --headed scroll $anchor.data.match.ref_id --direction down --amount 10
        if not $scrolled.ok { error make {msg: 'Could not scroll the file action list'} }
        let confirm = log-count $log 'dupin-trace: action-screen confirm'
        click-file-action $out file-actions-trash-review 'Move to Trash'
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
        let list = log-count $log 'dupin-trace: action-screen list'
        ad $out file-actions-trash-cancel press return | ignore
        wait-for-log --after $list $log 'dupin-trace: action-screen list'
        let confirm = log-count $log 'dupin-trace: action-screen confirm'
        (click-file-action
            $out
            file-actions-trash-review-again
            'Move to Trash'
        )
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

def click-file-action [out: path, name: string, title: string] {
    let action = find-file-action $out $'($name)-button' $title
    let clicked = ad $out $name click $action.data.match.ref_id
    if not $clicked.ok { error make {msg: $'Could not activate ($title)'} }
}

def find-file-action [out: path, name: string, title: string]: nothing -> record {
    (poll
        $out
        $name
        {|r|
            let label = $r.data?.match?.name? | default '' | str replace --regex '\s*↩$' '' | str trim
            $r.ok and $r.data?.match?.ref_id? != null and $label == $title
        }
        find --app Dupin --role button --name $title --first
    )
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
    let right = log-count $log 'dupin-trace: key Right '
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
