use driver.nu [
    DUPIN
    ad
    file-proof
    log-count
    mentions
    poll
    press
    snap
    wait-for-log
]
use ../e2e-state.nu is-focused-query

export def cli-stage [out: path, binary: path] {
    let recording = $out | path join recording
    if ($recording | path join started.json | path exists) {
        let started = open ($recording | path join started.json) | into datetime
        ((date now) - $started) / 1sec | to json | save --force ($recording | path join cli-start.json)
    }
    let log = $out | path join dupin.log
    let archive = $out | path join data dupin snapshots archive.json
    let history = $env.HOME | path join 'Library/Application Support/Dupin/history.json'
    let before = file-proof $history
    let archive_before = file-proof $archive
    try {
        let opened = ^$binary open --query calculator --json | from json
        if $opened != null { error make {msg: 'CLI open returned unexpected data'} }
        wait-for-log $log 'dupin-trace: control open ok=true'
        (poll
            $out
            cli-open
            {|r| is-focused-query $r calculator }
            find
            --app
            $DUPIN
            --role
            textfield
            --first
        ) | ignore
        snap $out cli-open
        let closed = log-count $log 'dupin-trace: panel closed '
        press $out cli-hide escape | ignore
        wait-for-log --after $closed $log 'dupin-trace: panel closed '

        let selected_closed = log-count $log 'dupin-trace: panel closed '
        job spawn { "red\nblue\n" | ^$binary pick --json | complete | job send 0 --tag 2 } | ignore
        (poll
            $out
            cli-pick
            {|r| is-focused-query $r '' }
            find
            --app
            $DUPIN
            --native-id
            pick-query
            --first
        ) | ignore
        (poll
            $out
            cli-picker-panel
            {|r| $r.ok and (
                $r.data?.tree?.children?
                | default []
                | any {|window|
                    let fits = $window.role == dialog and ($window.bounds?.height? | default 0) >= 168
                    $fits and (mentions {data: $window} red) and (mentions {data: $window} blue)
                }
            ) }
            snapshot
            --app
            $DUPIN
            --include-bounds
        ) | ignore
        snap $out cli-pick
        press $out cli-pick-blue down return | ignore
        let selected = job recv --tag 2 --timeout 20sec
        $selected | to json | save ($out | path join cli-picked.json)
        if $selected.exit_code != 0 or ($selected.stdout | from json) != blue {
            error make {msg: 'CLI picker did not return the original blue value'}
        }
        wait-for-log $log 'dupin-trace: control pick ok=true'
        wait-for-log --after $selected_closed $log 'dupin-trace: panel closed '
        let closed = log-count $log 'dupin-trace: panel closed '
        job spawn { "red\nblue\n" | ^$binary pick --json | complete | job send 0 --tag 3 } | ignore
        (poll
            $out
            cli-cancel-field
            {|r| is-focused-query $r '' }
            find
            --app
            $DUPIN
            --native-id
            pick-query
            --first
        ) | ignore
        press $out cli-cancel escape | ignore
        let result = job recv --tag 3 --timeout 20sec
        $result | to json | save ($out | path join cli-cancelled.json)
        if $result.exit_code != 1 or ($result.stdout | from json).error != Cancelled {
            error make {msg: 'CLI picker cancellation did not return Cancelled'}
        }
        wait-for-log --after $closed $log 'dupin-trace: panel closed '
        (poll
            $out
            cli-hidden
            {|r| $r.ok and $r.data?.match? == null }
            find
            --app
            $DUPIN
            --role
            textfield
            --first
        ) | ignore

        let listed = ^$binary snapshots list --json | from json
        $listed | to json | save ($out | path join cli-snapshots.json)
        if ($listed.entries | length) != 1 or $listed.truncated {
            error make {msg: 'CLI Snapshot list did not return the saved fixture'}
        }
        if ($listed.entries.0 | columns | sort) != ([id mode query saved_at title] | sort) {
            error make {msg: 'CLI Snapshot list exposed unexpected fields'}
        }
        let id = $listed.entries.0.id
        let opened = ^$binary snapshots open --id $id --json | from json
        if $opened != null { error make {msg: 'CLI Snapshot open returned unexpected data'} }
        poll $out cli-snapshot-window {|r| $r.ok and (
            $r.data?
            | default []
            | any {|w| $w.title == 'Dupin Snapshots' and $w.is_focused }
        ) } list-windows --app $DUPIN | ignore
        (poll
            $out
            cli-snapshot-field
            {|r| is-focused-query $r '' }
            find
            --app
            $DUPIN
            --role
            textfield
            --name
            'Search saved snapshots'
            --exact
            --first
        ) | ignore
        snap $out cli-snapshots-open
        let closed = ^$binary snapshots close --json | from json
        if $closed != null { error make {msg: 'CLI Snapshot close returned unexpected data'} }
        (poll
            $out
            cli-snapshot-closed
            {|r| $r.ok and not ($r.data? | default [] | any {|w| $w.title == 'Dupin Snapshots' }) }
            list-windows
            --app
            $DUPIN
        ) | ignore
        let panel_closed = log-count $log 'dupin-trace: panel closed '
        ^$binary open --query calculator --json | from json | ignore
        (poll
            $out
            cli-before-snapshot-handoff
            {|r| is-focused-query $r calculator }
            find
            --app
            $DUPIN
            --role
            textfield
            --first
        ) | ignore
        ^$binary snapshots open --id $id --json | from json | ignore
        ^$binary snapshots close --json | from json | ignore
        wait-for-log --after $panel_closed $log 'dupin-trace: panel closed '
        (poll
            $out
            cli-snapshot-handoff-closed
            {|r| $r.ok and not ($r.data? | default [] | any {|w| $w.title == 'Dupin Snapshots' }) }
            list-windows
            --app
            $DUPIN
        ) | ignore
        if (file-proof $archive) != $archive_before or (file-proof $history) != $before {
            error make {msg: 'CLI controls changed the Snapshot archive or launch history'}
        }
        {
            archive_before: $archive_before
            archive_after: (file-proof $archive)
            history_before: $before
            history_after: (file-proof $history)
        } | to json | save ($out | path join cli-storage-proof.json)
    } catch {|failure|
        try { snap $out cli-failure }
        error make $failure
    }
}
