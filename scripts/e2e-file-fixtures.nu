const registration = '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'
const fixtures = path self fixtures

export def viewer-running []: nothing -> bool {
    let process = ^pgrep -f 'DupinE2EViewer.app/Contents/MacOS/viewer' | complete
    match $process.exit_code {
        0 => true
        1 => false
        _ => { error make {msg: 'Could not check the fixture viewer process'} }
    }
}

export def readable-trash [] {
    let result = ^ls -A ($env.HOME | path join .Trash) | complete
    if $result.exit_code != 0 {
        error make {
            msg: 'Cannot read the disposable runner Trash; file-action verification requires access'
        }
    }
}

export def prepare [out: path] {
    let root = $out | path join fixtures
    if ($root | path exists) { error make {msg: 'File-action fixtures already exist'} }
    let bundle = $root | path join DupinE2EViewer.app
    mkdir ($bundle | path join Contents MacOS)
    cp ($fixtures | path join FileViewer.plist) ($bundle | path join Contents Info.plist)
    ^swiftc -framework AppKit ($fixtures | path join FileViewer.swift) -o ($bundle | path join Contents MacOS viewer)
    ^codesign --force --sign - $bundle
    ^$registration -f $bundle
    for name in [open-me.dupine2e trash-me.dupine2e] {
        $'Dupin disposable fixture: ($root)/($name)' | save ($root | path join $name)
    }
    open --raw ($root | path join trash-me.dupine2e) | hash sha256 | save ($root | path join trash.sha256)
    let imported = ^mdimport $root | complete
    if $imported.exit_code != 0 {
        error make {msg: $'Fixture import failed: ($imported.stderr)'}
    }
    mut found = {exit_code: 0, stdout: '', stderr: ''}
    for _ in 1..30 {
        $found = (^mdfind -onlyin $root 'kMDItemFSName == "*.dupine2e"' | complete)
        let indexed = $found.stdout | lines
        if $found.exit_code == 0 and ($indexed | all {|path| $path | path exists }) and ($indexed | length) == 2 { return }
        sleep 1sec
    }
    {
        volume: (^mdutil -s $root | complete)
        import: $imported
        search: $found
        name_search: (^mdfind -onlyin $root -name dupine2e | complete)
        fixtures: ([open-me.dupine2e trash-me.dupine2e] | each {|name|
            let file = $root | path join $name
            {
                path: $file
                metadata: (^mdls -name kMDItemContentType -name kMDItemFSName $file | complete)
                test_import: (^mdimport -t -d1 $file | complete)
            }
        })
    } | to nuon --indent 2 | save ($out | path join spotlight-diagnosis.txt)
    error make {
        msg: 'Spotlight did not index both file-action fixtures; see spotlight-diagnosis.txt'
    }
}

export def cleanup [out: path] {
    let bundle = $out | path join fixtures DupinE2EViewer.app
    let failures = [
        (try {
            if (viewer-running) {
                ^osascript -e 'tell application id "org.dupin.e2e.viewer" to quit'
            }
            null
        } catch {|error| $error })
        (try {
            ^$registration -u $bundle
            null
        } catch {|error|
            $error
        })
        (try {
            readable-trash
            for path in (glob ($env.HOME | path join '.Trash/trash-me*.dupine2e')) {
                let expected = open --raw ($out | path join fixtures trash.sha256)
                if (open --raw $path | hash sha256) != $expected {
                    error make {msg: 'Refusing to remove an unexpected Trash item'}
                }
                rm $path
            }
            null
        } catch {|error| $error })
    ] | where {|error| $error != null }
    if ($failures | is-not-empty) {
        error make ($failures | first)
    }
}
