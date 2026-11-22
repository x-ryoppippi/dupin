#!/usr/bin/env nu

def assert [condition: bool, message: string] {
    if not $condition { error make {msg: $message} }
}

let root = mktemp --directory | path expand

let failure = try {
    let bin = $root | path join bin
    mkdir $bin
    let gh = $bin | path join gh
    cp scripts/tests/fixtures/e2e-gh.nu $gh
    ^chmod +x $gh
    let fixtures = $root | path join fixtures
    mkdir $fixtures
    0x[89 50 4E 47 0D 0A 1A 0A] | save ($fixtures | path join frame.png)
    0x[00 00 00 0C 66 74 79 70 69 73 6F 6D] | save ($fixtures | path join clip.mp4)
    let report = $root | path join report.bash
    open .github/workflows/e2e-comment.yml | get jobs.comment.steps | last | get run | save $report
    let cases = [
        {
            name: pr
            event: pull_request
            token: true
            failed: false
            existing: true
            legacy: false
            stale: false
        }
        {
            name: commit
            event: push
            token: true
            failed: false
            existing: false
            legacy: false
            stale: false
        }
        {
            name: no-token
            event: pull_request
            token: false
            failed: false
            existing: false
            legacy: true
            stale: false
        }
        {
            name: upload-failure
            event: push
            token: true
            failed: true
            existing: true
            legacy: false
            stale: false
        }
        {
            name: stale-pr
            event: pull_request
            token: true
            failed: false
            existing: false
            legacy: false
            stale: true
        }
    ]
    for case in $cases {
        let work = $root | path join $case.name
        mkdir $work
        let posted = $work | path join posted.md
        let calls = $work | path join calls
        let run = with-env {
            PATH: ($env.PATH | prepend $bin)
            GH_TOKEN: fixture-comment-token
            ASSET_TOKEN: (if $case.token { 'fixture-upload-token' } else { '' })
            GH_UPLOAD_FAIL: (if $case.failed { '1' } else { '0' })
            GH_EXISTING: (if $case.existing { '1' } else { '0' })
            GH_LEGACY: (if $case.legacy { '1' } else { '0' })
            GH_STALE: (if $case.stale { '1' } else { '0' })
            GH_FIXTURES: $fixtures
            GH_POSTED: $posted
            GH_CALLS: $calls
            REPO: owner/repo
            RUN_ID: '42'
            RUN_URL: 'https://github.com/owner/repo/actions/runs/42'
            EVENT: $case.event
            SHA: '1234567890123456789012345678901234567890'
            HEAD_OWNER: owner
            HEAD_BRANCH: feature
            RUNNER_TEMP: $work
        } { ^bash --noprofile --norc $report | complete }
        if $run.exit_code != 0 { print --stderr $run.stderr }
        assert ($run.exit_code == 0) $'reporting failed: ($case.name)'
        let commands = open --raw $calls
        if $case.stale {
            assert (not ($posted | path exists)) 'a stale PR run must not post'
            assert (not ($commands | str contains 'uploads.github.com')) 'a stale PR run must not upload'
        } else {
            let body = open --raw $posted
            assert ($body | str starts-with '<!-- dupin-e2e -->') 'report marker must survive'
            assert ($body | str contains 'E2E passed on 1234567') 'report must preserve the tested commit and result'
            assert ($body | str contains '[Log and detailed evidence]') 'report must retain the logs'
            if $case.token and not $case.failed {
                assert (
                    $body
                    | str contains '![E2E frame.png](https://github.com/user-attachments/assets/'
                ) 'images must render inline'
                assert (
                    $body
                    | str contains "\nhttps://github.com/user-attachments/assets/12345678-1234-1234-1234-123456789def\n"
                ) 'video must use a standalone player URL'
                assert (not ($body | str contains 'Download media')) 'successful inline media must replace the ZIP link'
            } else {
                assert ($body | str contains '[Download media]') 'upload failure must retain downloadable evidence'
                if not $case.token {
                    assert (not ($commands | str contains 'uploads.github.com')) 'missing credentials must not attempt upload'
                }
            }
            let route = if $case.event == 'pull_request' {
                if $case.existing { 'repos/owner/repo/issues/comments/17' } else { 'repos/owner/repo/issues/7/comments' }
            } else {
                if $case.existing { 'repos/owner/repo/comments/17' } else { 'repos/owner/repo/commits/1234567890123456789012345678901234567890/comments' }
            }
            assert ($commands | str contains $route) $'wrong comment target: ($case.name)'
        }
    }
    null
} catch {|error|
    $error | to nuon
}
rm --recursive --force $root
if $failure != null {
    print $failure
    error make {msg: 'E2E comment workflow tests failed'}
}
print 'E2E comment workflow tests passed'
