use driver.nu [
    ad
    file-proof
    log-count
    poll
    press
    snap
    type-text
    wait-for-log
]
use ../e2e-state.nu [has-match is-focused-query]

export def snapshot-stage [out: path] {
    let found = {|r| has-match $r }
    let log = $out | path join dupin.log
    let archive = $out | path join data dupin snapshots archive.json
    let history = $env.HOME | path join 'Library/Application Support/Dupin/history.json'
    let before = file-proof $history
    $before | to json | save --force ($out | path join snapshots-history-before.json)
    if ($archive | path exists) { error make {msg: 'Snapshot archive was not isolated'} }
    try {
        let done = log-count $log 'dupin-trace: results done query="calculator"'
        type-text $out snapshots-type-calculator calculator | ignore
        (poll
            $out
            snapshots-query
            {|r| is-focused-query $r calculator }
            find
            --app
            Dupin
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
            Dupin
            --name
            'Calculator /System/Applications'
            --exact
            --first
        ) | ignore
        press $out snapshots-save cmd+shift+s | ignore
        wait-for-log $log 'dupin-trace: snapshot capture accepted'
        wait-for-log $log 'dupin-trace: snapshot saved'
        let saved = open $archive
        if ($saved.entries | length) != 1 or $saved.entries.0.mode != 'apps.search' or $saved.entries.0.query != calculator {
            error make {msg: 'Snapshot did not capture the settled Apps query'}
        }
        let archive_before = file-proof $archive
        cp $archive ($out | path join snapshots-archive-after-save.json)
        snap $out snapshots-saved

        let closed = log-count $log 'dupin-trace: panel closed '
        press $out snapshots-open cmd+shift+o | ignore
        wait-for-log --after $closed $log 'dupin-trace: panel closed '
        poll $out snapshots-window {|r| $r.ok and (
            $r.data?
            | default []
            | any {|w| $w.title == 'Dupin Snapshots' and $w.is_focused }
        ) } list-windows --app Dupin | ignore
        (poll
            $out
            snapshots-field
            {|r| is-focused-query $r '' }
            find
            --app
            Dupin
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
            Dupin
            --name
            'Historical snapshot · saved content'
            --exact
            --first
        ) | ignore
        snap $out snapshots-opened

        type-text $out snapshots-type-filter zzz | ignore
        (poll
            $out
            snapshots-filter
            {|r| is-focused-query $r zzz }
            find
            --app
            Dupin
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
            Dupin
            --name
            'No matches for this search.'
            --exact
            --first
        ) | ignore
        snap $out snapshots-filtered
        press $out snapshots-clear-filter cmd+backspace | ignore
        (poll
            $out
            snapshots-cleared
            {|r| is-focused-query $r '' }
            find
            --app
            Dupin
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
            Dupin
            --name
            'Historical snapshot · saved content'
            --exact
            --first
        ) | ignore
        if (file-proof $archive) != $archive_before { error make {msg: 'Editing snapshot search changed the archive'} }
        snap $out snapshots-search-cleared

        let delete = (poll
            $out
            snapshots-delete
            $found
            find
            --app
            Dupin
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
            Dupin
            --name
            $prompt
            --exact
            --first
        ) | ignore
        snap $out snapshots-confirm
        if not (open --raw ($out | path join snapshots-confirm-snapshot.json) | str contains $prompt) {
            error make {msg: 'Snapshot tree omitted the visible delete confirmation'}
        }
        let cancel = (poll
            $out
            snapshots-cancel
            $found
            find
            --app
            Dupin
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
            Dupin
        ) | ignore
        (poll
            $out
            snapshots-delete-restored
            $found
            find
            --app
            Dupin
            --role
            button
            --name
            'Delete…'
            --exact
            --first
        ) | ignore
        if (file-proof $archive) != $archive_before { error make {msg: 'Cancelling snapshot deletion changed the archive'} }
        cp $archive ($out | path join snapshots-archive-after-cancel.json)
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
            {|r| $r.ok and not ($r.data? | default [] | any {|w| $w.title == 'Dupin Snapshots' }) }
            list-windows
            --app
            Dupin
        ) | ignore
        press $out snapshots-summon alt+space | ignore
        (poll
            $out
            snapshots-panel
            {|r| is-focused-query $r '' }
            find
            --app
            Dupin
            --role
            textfield
            --first
        ) | ignore
        snap $out snapshots-resummoned
        let after = file-proof $history
        $after | to json | save --force ($out | path join snapshots-history-after.json)
        if $after != $before { error make {msg: 'Snapshot flow changed launch history'} }
    } catch {|failure|
        try { snap $out snapshots-failure }
        try { cp $log ($out | path join snapshots-failure.log) }
        error make $failure
    }
}
