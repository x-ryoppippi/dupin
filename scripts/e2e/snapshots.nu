use driver.nu [
    DUPIN
    ad
    assert-sealed
    dir-proof
    file-proof
    log-count
    poll
    press
    evidence
    type-text
    wait-for-log
]
use ../e2e-state.nu [has-match is-focused-query]

export def snapshot-stage [out: path] {
    let found = {|r| has-match $r }
    let log = $out | path join dupin.log
    let data = $out | path join data
    let archive = $data | path join dupin storage snapshots
    let history = $env.HOME | path join 'Library/Application Support/Dupin/history.json'
    let before = file-proof $history
    $before | to json | save --force ($out | path join snapshots-history-before.json)
    if ($archive | path exists) { error make {msg: 'Snapshot storage was not isolated'} }
    try {
        let done = log-count $log 'dupin-trace: results done query="calculator"'
        type-text $out snapshots-type-calculator calculator | ignore
        (poll
            $out
            snapshots-query
            {|r| is-focused-query $r calculator }
            find
            --app
            $DUPIN
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
            $DUPIN
            --name
            'Calculator /System/Applications'
            --exact
            --first
        ) | ignore
        press $out snapshots-save cmd+shift+s | ignore
        wait-for-log $log 'dupin-trace: snapshot capture accepted'
        wait-for-log $log 'dupin-trace: snapshot saved'
        let saved = glob ($archive | path join records '*.rec')
        if ($saved | length) != 1 {
            error make {msg: 'Snapshot did not save one sealed record'}
        }
        assert-sealed $data calculator
        assert-sealed $data 'Calculator /System/Applications'
        let archive_before = dir-proof $archive
        $archive_before | to json | save --force ($out | path join snapshots-archive-after-save.json)
        evidence $out snapshots-saved

        let closed = log-count $log 'dupin-trace: panel closed '
        press $out snapshots-open cmd+shift+o | ignore
        wait-for-log --after $closed $log 'dupin-trace: panel closed '
        poll $out snapshots-window {|r| $r.ok and (
            $r.data?
            | default []
            | any {|w| $w.title == 'Dupin Settings' and $w.is_focused }
        ) } list-windows --app $DUPIN | ignore
        (poll
            $out
            snapshots-field
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
        (poll
            $out
            snapshots-detail
            $found
            find
            --app
            $DUPIN
            --name
            'Historical snapshot · saved content'
            --exact
            --first
        ) | ignore
        evidence $out snapshots-opened

        type-text $out snapshots-type-filter zzz | ignore
        (poll
            $out
            snapshots-filter
            {|r| is-focused-query $r zzz }
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
        (poll
            $out
            snapshots-no-matches
            $found
            find
            --app
            $DUPIN
            --name
            'No matches for this search.'
            --exact
            --first
        ) | ignore
        evidence $out snapshots-filtered
        press $out snapshots-clear-filter cmd+backspace | ignore
        (poll
            $out
            snapshots-cleared
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
        (poll
            $out
            snapshots-restored
            $found
            find
            --app
            $DUPIN
            --name
            'Historical snapshot · saved content'
            --exact
            --first
        ) | ignore
        if (dir-proof $archive) != $archive_before { error make {msg: 'Editing snapshot search changed the archive'} }
        evidence $out snapshots-search-cleared

        let delete = (poll
            $out
            snapshots-delete
            $found
            find
            --app
            $DUPIN
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
            $DUPIN
            --name
            $prompt
            --exact
            --first
        ) | ignore
        evidence $out snapshots-confirm
        if not (open --raw ($out | path join snapshots-confirm-snapshot.json) | str contains $prompt) {
            error make {msg: 'Snapshot tree omitted the visible delete confirmation'}
        }
        let cancel = (poll
            $out
            snapshots-cancel
            $found
            find
            --app
            $DUPIN
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
            $DUPIN
        ) | ignore
        (poll
            $out
            snapshots-delete-restored
            $found
            find
            --app
            $DUPIN
            --role
            button
            --name
            'Delete…'
            --exact
            --first
        ) | ignore
        if (dir-proof $archive) != $archive_before { error make {msg: 'Cancelling snapshot deletion changed the archive'} }
        dir-proof $archive | to json | save --force ($out | path join snapshots-archive-after-cancel.json)
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
        press $out snapshots-close cmd+w | ignore
        (poll
            $out
            snapshots-closed
            {|r| $r.ok and not ($r.data? | default [] | any {|w| $w.title == 'Dupin Settings' }) }
            list-windows
            --app
            $DUPIN
        ) | ignore
        press $out snapshots-summon alt+space | ignore
        (poll
            $out
            snapshots-panel
            {|r| is-focused-query $r '' }
            find
            --app
            $DUPIN
            --role
            textfield
            --first
        ) | ignore
        evidence $out snapshots-resummoned
        let after = file-proof $history
        $after | to json | save --force ($out | path join snapshots-history-after.json)
        if $after != $before { error make {msg: 'Snapshot flow changed launch history'} }
    } catch {|failure|
        try { evidence $out snapshots-failure }
        try { cp $log ($out | path join snapshots-failure.log) }
        error make $failure
    }
}
