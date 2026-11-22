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
    if ($request | str contains '/zip') {
        if $env.GH_TOKEN != 'fixture-comment-token' { exit 22 }
        if not ($request | str contains '/artifacts/10/zip') { exit 26 }
        open --raw ($env.GH_FIXTURES | path join media.zip) | print --raw --no-newline
        return
    }
    if ($request | str contains 'body=@') {
        if $env.GH_TOKEN != 'fixture-comment-token' { exit 23 }
        let body = $args | where {|arg| $arg | str starts-with 'body=@'} | first | str substring 6..
        cp $body $env.GH_POSTED
        return
    }
    if ($request | str contains '/artifacts') {
        open --raw $env.GH_ARTIFACTS | print --raw
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
