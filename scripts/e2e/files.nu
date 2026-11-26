use driver.nu [
    DUPIN
    ad
    log-count
    mentions
    poll
    press
    evidence
    type-text
    wait-condition
    wait-for-log
]
use ../e2e-file-fixtures.nu [documents trashed viewer-running]
use ../e2e-state.nu [has-match is-focused-query]

export def file-actions-stage [out: path] {
    let found = {|r| has-match $r }
    let log = $out | path join dupin.log
    let root = $out | path join fixtures
    let open_path = documents $out | path join open-me.dupine2e
    let trash_path = documents $out | path join trash-me.dupine2e
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
            $DUPIN
            --name
            'DupinE2EViewer Default'
            --exact
            --first
        )
        evidence $out file-actions-chooser
        ad $out file-actions-open click $viewer.data.match.ref_id | ignore
        let marker = $root | path join opened.json
        wait-condition 'fixture application opening the exact path' {
            ($marker | path exists) and (open $marker) == [$open_path]
        }
        evidence $out file-actions-opened
        ^osascript -e 'tell application id "org.dupin.e2e.viewer" to quit'
        wait-condition 'fixture viewer quitting' { not (viewer-running) }

        file-search $out trash-me $found
        let confirm = log-count $log 'dupin-trace: action-screen confirm'
        open-trash-confirmation $out file-actions-trash-review
        wait-for-log --after $confirm $log 'dupin-trace: action-screen confirm'
        (poll
            $out
            file-actions-trash-question
            $found
            find
            --app
            $DUPIN
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
            $DUPIN
            --name
            $trash_path
            --exact
            --first
        ) | ignore
        evidence $out file-actions-trash-confirm
        let list = log-count $log 'dupin-trace: action-screen list'
        press $out file-actions-trash-cancel return | ignore
        wait-for-log --after $list $log 'dupin-trace: action-screen list'
        let confirm = log-count $log 'dupin-trace: action-screen confirm'
        open-trash-confirmation $out file-actions-trash-review-again
        wait-for-log --after $confirm $log 'dupin-trace: action-screen confirm'
        if not ($trash_path | path exists) { error make {msg: 'Enter on Cancel removed the fixture'} }
        evidence $out file-actions-trash-cancelled
        press $out file-actions-trash-select down | ignore
        press $out file-actions-trash-confirm return | ignore
        wait-condition 'fixture moving to the Trash' {
            not ($trash_path | path exists) and (trashed $out | length) == 1
        }
        let moved = trashed $out | first
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
            $DUPIN
        ) | ignore
        (poll
            $out
            file-actions-trash-notice
            $found
            find
            --app
            $DUPIN
            --name
            'Moved “trash-me.dupine2e” to the Trash'
            --exact
            --first
        ) | ignore
        evidence $out file-actions-trash-finished
        press $out file-actions-close escape | ignore
    } catch {|failure|
        try { evidence $out file-actions-failure }
        try { cp $log ($out | path join file-actions-failure.log) }
        error make $failure
    }
}

def open-trash-confirmation [out: path, name: string] {
    mut selected = ''
    # Down reveals the next row, whose accessibility label includes the Return marker.
    for step in 0..<20 {
        let previous = $selected
        let row = (poll
            $out
            $'($name)-selection-($step)'
            {|r| (has-match $r) and $r.data.match.name != $previous }
            find --app $DUPIN --role button --name '↩' --first
        )
        $selected = $row.data.match.name
        if $selected == 'Move to Trash ↩' {
            press $out $name return | ignore
            return
        }
        press $out $'($name)-down-($step)' down | ignore
    }
    error make {msg: 'Could not select Move to Trash'}
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
        find --app $DUPIN --role button --name $title --first
    )
}

def file-row-name [out: path, name: string]: nothing -> string {
    $'($name).dupine2e (file-parent $out)'
}

def file-parent [out: path]: nothing -> string {
    let parent = documents $out
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
    press $out $'file-actions-($query)-summon' alt+space | ignore
    (poll
        $out
        $'file-actions-($query)-panel'
        {|r| is-focused-query $r '' }
        find
        --app
        $DUPIN
        --role
        textfield
        --first
    ) | ignore
    press $out $'file-actions-($query)-space' space | ignore
    (poll
        $out
        $'file-actions-($query)-mode'
        {|r| is-focused-query $r '' }
        find
        --app
        $DUPIN
        --role
        textfield
        --name
        'Search files'
        --exact
        --first
    ) | ignore
    type-text $out $'file-actions-($query)-type' $file | ignore
    (poll
        $out
        $'file-actions-($query)-query'
        {|r| is-focused-query $r $file }
        find
        --app
        $DUPIN
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
        $DUPIN
        --name
        (file-row-name $out $query)
        --exact
        --first
    ) | ignore
    let right = log-count $log 'dupin-trace: key Right '
    press $out $'file-actions-($query)-actions' right | ignore
    wait-for-log --after $right $log 'dupin-trace: key Right '
    (poll
        $out
        $'file-actions-($query)-context-title'
        $found
        find
        --app
        $DUPIN
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
        $DUPIN
        --name
        $parent
        --exact
        --first
    ) | ignore
}
