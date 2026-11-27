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

# The runners keep Spotlight to filesystem-only search, which on some VMs finds nothing on the
# boot data volume for the VM's whole life, so the documents get a volume of their own.
export def documents [out: path]: nothing -> string {
    $out | path join fixtures documents
}

export def trashed [out: path]: nothing -> list<string> {
    glob (documents $out | path join .Trashes (^id -u | str trim) 'trash-me*.dupine2e')
}

export def prepare [out: path] {
    let root = $out | path join fixtures
    if ($root | path exists) { error make {msg: 'File-action fixtures already exist'} }
    let documents = documents $out
    mkdir $documents
    let image = $root | path join documents.dmg
    ^hdiutil create -quiet -size 16m -fs APFS -volname DupinE2E $image
    ^hdiutil attach -quiet -nobrowse -mountpoint $documents $image
    let bundle = $root | path join DupinE2EViewer.app
    mkdir ($bundle | path join Contents MacOS)
    cp ($fixtures | path join FileViewer.plist) ($bundle | path join Contents Info.plist)
    ^swiftc -framework AppKit ($fixtures | path join FileViewer.swift) -o ($bundle | path join Contents MacOS viewer)
    ^codesign --force --sign - $bundle
    ^$registration -f $bundle
    for name in [open-me.dupine2e trash-me.dupine2e] {
        $'Dupin disposable fixture: ($documents)/($name)' | save ($documents | path join $name)
    }
    open --raw ($documents | path join trash-me.dupine2e) | hash sha256 | save ($root | path join trash.sha256)
    mut found = {exit_code: 0, stdout: '', stderr: ''}
    for _ in 1..30 {
        $found = (^mdfind -onlyin $documents 'kMDItemFSName == "*.dupine2e"' | complete)
        let listed = $found.stdout | lines
        if $found.exit_code == 0 and ($listed | all {|path| $path | path exists }) and ($listed | length) == 2 { return }
        sleep 1sec
    }
    {
        volume: (^mdutil -s $documents | complete)
        search: $found
        name_search: (^mdfind -onlyin $documents -name dupine2e | complete)
        fixtures: ([open-me.dupine2e trash-me.dupine2e] | each {|name|
            let file = $documents | path join $name
            {
                path: $file
                metadata: (^mdls -name kMDItemContentType -name kMDItemFSName $file | complete)
            }
        })
    } | to nuon --indent 2 | save ($out | path join spotlight-diagnosis.txt)
    error make {
        msg: 'Spotlight did not find both file-action fixtures; see spotlight-diagnosis.txt'
    }
}

export def cleanup [out: path] {
    let root = $out | path join fixtures
    let documents = documents $out
    let failures = [
        (try {
            if (viewer-running) {
                ^osascript -e 'tell application id "org.dupin.e2e.viewer" to quit'
            }
            null
        } catch {|error| $error })
        (try {
            ^$registration -u ($root | path join DupinE2EViewer.app)
            null
        } catch {|error|
            $error
        })
        (try {
            if (^mount | lines | parse '{device} on {path} ({options})' | any {|mount| $mount.path == $documents }) {
                ^hdiutil detach -quiet $documents
            }
            rm --force ($root | path join documents.dmg)
            null
        } catch {|error| $error })
    ] | where {|error| $error != null }
    if ($failures | is-not-empty) {
        error make ($failures | first)
    }
}
