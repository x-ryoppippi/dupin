#!/usr/bin/env -S nu --no-config-file

def --wrapped main [...args: string] {
    let request = $args | str join ' '
    $"($request)\n" | save --append $env.GH_CALLS
    if ($request | str contains 'uploads.github.com') {
        if $env.GH_TOKEN != 'fixture-upload-token' { exit 20 }
        if $env.GH_UPLOAD_FAIL == '1' { exit 21 }
        let suffix = if ($request | str contains 'name=clip.mp4') { 'def' } else { 'abc' }
        print (
            {
                url: $'https://github.com/user-attachments/assets/12345678-1234-1234-1234-123456789($suffix)'
            }
            | to json --raw
        )
        return
    }
    if ($request | str contains '--jq .id') {
        if $env.GH_TOKEN != 'fixture-upload-token' { exit 25 }
        print '123'
        return
    }
    if ($request | str contains 'run download') {
        if $env.GH_TOKEN != 'fixture-comment-token' { exit 22 }
        let at = $args | enumerate | where item == '--dir' | get 0.index
        let directory = $args | get ($at + 1)
        cp ($env.GH_FIXTURES | path join frame.png) ($env.GH_FIXTURES | path join clip.mp4) $directory
        return
    }
    if ($request | str contains 'body=@') {
        if $env.GH_TOKEN != 'fixture-comment-token' { exit 23 }
        let body = $args | where {|arg| $arg | str starts-with 'body=@'} | first | str substring 6..
        cp $body $env.GH_POSTED
        return
    }
    if ($request | str contains '/artifacts') {
        if ($request | str contains 'e2e-media') {
            if $env.GH_LEGACY != '1' { print 'https://github.com/owner/repo/actions/runs/42/artifacts/10' }
        } else if ($request | str contains 'e2e-screenshots') {
            if $env.GH_LEGACY == '1' { print 'https://github.com/owner/repo/actions/runs/42/artifacts/11' }
        } else {
            print 'https://github.com/owner/repo/actions/runs/42/artifacts/12'
        }
        return
    }
    if ($request | str contains '/jobs') {
        print 'success'
        return
    }
    if ($request | str contains 'pulls?state=open') {
        if $env.GH_LATE_STALE == '1' and (open --raw $env.GH_CALLS | str contains 'uploads.github.com') { return }
        if $env.GH_STALE != '1' { print '7' }
        return
    }
    if ($request | str contains '/comments') {
        if $env.GH_EXISTING == '1' { print '17' }
        return
    }
    exit 24
}
