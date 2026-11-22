#!/usr/bin/env nu

use ../e2e-assets.nu [inspect-assets]

def assert [condition: bool, message: string] {
    if not $condition {
        error make {msg: $message}
    }
}

def write-bytes [directory: path, name: string, data: binary] {
    $data | save --force ($directory | path join $name)
}

def expect-rejection [directory: path, label: string] {
    let rejected = try {
        inspect-assets $directory
        false
    } catch {
        true
    }
    assert $rejected $'expected rejection: ($label)'
}

let temp_root = $env.TMPDIR? | default '/tmp'
let work = $temp_root | path join $'dupin-e2e-assets-($nu.pid)-(random uuid)'
mkdir $work

let failure = try {
    let valid = $work | path join 'valid'
    mkdir $valid

    write-bytes $valid 'frame.png' 0x[89 50 4E 47 0D 0A 1A 0A]
    write-bytes $valid 'frame.jpg' 0x[FF D8 FF]
    write-bytes $valid 'frame.jpeg' 0x[FF D8 FF]
    write-bytes $valid 'frame.gif' ('GIF89a' | into binary)
    write-bytes $valid 'frame.webp' 0x[52 49 46 46 00 00 00 00 57 45 42 50]
    write-bytes $valid 'run.mp4' 0x[00 00 00 0C 66 74 79 70 69 73 6F 6D]
    write-bytes $valid 'run.mov' 0x[00 00 00 0C 66 74 79 70 71 74 20 20]
    write-bytes $valid 'run.webm' 0x[1A 45 DF A3]
    'runner output' | save ($valid | path join 'runner.log')
    ^ln -s ($valid | path join 'frame.png') ($valid | path join 'linked.png')

    let assets = inspect-assets $valid
    assert (($assets | length) == 8) 'manifest should include all eight supported media extensions'
    assert (($assets | where name == 'linked.png' | length) == 0) 'manifest must ignore symlinks'
    assert (($assets | where name == 'runner.log' | length) == 0) 'manifest must ignore non-media files'
    assert ((
        $assets
        | where name == 'run.mp4'
        | get mime_type
        | first
    ) == 'video/mp4') 'MP4 MIME type should be explicit'

    let corrupt = $work | path join 'corrupt'
    mkdir $corrupt
    write-bytes $corrupt 'bad.png' ('not a PNG' | into binary)
    expect-rejection $corrupt 'mismatched file signature'

    let unsafe = $work | path join 'unsafe'
    mkdir $unsafe
    write-bytes $unsafe 'bad name.png' 0x[89 50 4E 47 0D 0A 1A 0A]
    expect-rejection $unsafe 'unsafe filename'

    let too_many = $work | path join 'too-many'
    mkdir $too_many
    for index in 1..51 {
        (write-bytes
            $too_many
            $'frame-($index).png'
            0x[89 50 4E 47 0D 0A 1A 0A]
        )
    }
    expect-rejection $too_many 'more than 50 media files'

    let image_oversize = $work | path join 'image-oversize'
    mkdir $image_oversize
    let image = $image_oversize | path join 'large.png'
    ^truncate -s 10000001 $image
    expect-rejection $image_oversize 'image above 10 MB'

    let video_oversize = $work | path join 'video-oversize'
    mkdir $video_oversize
    let video = $video_oversize | path join 'large.mp4'
    ^truncate -s 100000001 $video
    expect-rejection $video_oversize 'video above 100 MB'

    let large_video = $work | path join 'large-video'
    mkdir $large_video
    let large_video_path = $large_video | path join 'recording.mp4'
    ^truncate -s 50000000 $large_video_path
    let signature = $work | path join 'mp4-signature.bin'
    (write-bytes
        $work
        'mp4-signature.bin'
        0x[00 00 00 0C 66 74 79 70 69 73 6F 6D]
    )
    ^dd $"if=($signature)" $"of=($large_video_path)" conv=notrunc
    let large_video_assets = inspect-assets $large_video
    assert (($large_video_assets | length) == 1) 'a 50 MB video should remain attachable'

    let fake_bin = $work | path join 'fake-bin'
    mkdir $fake_bin
    let marker = $work | path join 'gh-was-called'
    let fake_gh = $fake_bin | path join 'gh'
    [
        '#!/bin/sh'
        'printf called > "$GH_MARKER"'
        'exit 99'
        ''
    ] | str join (char newline) | save --force $fake_gh
    ^chmod +x $fake_gh

    let script = 'scripts/e2e-assets.nu' | path expand
    let nu_exe = $nu.current-exe
    let dry_run = with-env {
        PATH: ($env.PATH | prepend $fake_bin)
        GH_TOKEN: ''
        GH_MARKER: $marker
    } {
        ^$nu_exe $script owner/repository $valid | complete
    }
    assert ($dry_run.exit_code == 0) 'default invocation should produce a local manifest'
    assert (not ($marker | path exists)) 'dry-run must not call gh'
    let manifest = $dry_run.stdout | from json
    assert (not $manifest.uploaded) 'default invocation must be marked as not uploaded'
    assert (($manifest.assets | length) == 8) 'dry-run should return the media manifest'
    assert (($manifest.assets | where url != null | length) == 0) 'dry-run URLs must remain null'

    let no_token_upload = with-env {
        PATH: ($env.PATH | prepend $fake_bin)
        GH_TOKEN: ''
        GH_MARKER: $marker
    } {
        ^$nu_exe $script owner/repository $valid --upload | complete
    }
    assert ($no_token_upload.exit_code != 0) 'upload must require a caller-provided GH_TOKEN'
    assert (not ($marker | path exists)) 'upload without a token must fail before calling gh'

    let args_log = $work | path join 'upload-arguments'
    [
        '#!/bin/sh'
        'printf "%s\n" "$@" >> "$GH_ARGS_LOG"'
        'case "$*" in'
        '  *repos/owner/repository*) printf "123\n" ;;'
        "  *https://uploads.github.com/user-attachments/assets*) if [ \"${GH_INVALID_JSON:-}\" = 1 ]; then printf sensitive-service-response; else printf '{\"url\":\"%s\"}\\n' \"$GH_ASSET_URL\"; fi ;;"
        '  *) exit 98 ;;'
        'esac'
        ''
    ] | str join (char newline) | save --force $fake_gh
    let asset_url = 'https://github.com/user-attachments/assets/12345678-1234-1234-1234-123456789abc'
    let uploaded = with-env {
        PATH: ($env.PATH | prepend $fake_bin)
        GH_TOKEN: 'fixture-token'
        GH_ARGS_LOG: $args_log
        GH_ASSET_URL: $asset_url
    } {
        ^$nu_exe $script owner/repository $valid --upload | complete
    }
    assert ($uploaded.exit_code == 0) 'upload should use the raw GitHub attachment protocol'
    let uploaded_manifest = $uploaded.stdout | from json
    assert $uploaded_manifest.uploaded 'upload manifest should report uploaded media'
    assert (($uploaded_manifest.assets | length) == 8) 'upload should retain every validated asset'
    assert ($uploaded_manifest.assets | all {|asset| $asset.url == $asset_url}) 'upload should retain the returned attachment URL'
    let arguments = open --raw $args_log | lines
    assert ('--input' in $arguments) 'upload must send the file bytes through --input'
    assert ('Content-Type: application/octet-stream' in $arguments) 'upload must declare a raw body'
    assert ('repository_id=123' in $arguments) 'upload must bind assets to the repository ID'
    assert (($arguments | where $it == 'github.com' | length) == 9) 'all API calls must use github.com'

    let rejected_url = with-env {
        PATH: ($env.PATH | prepend $fake_bin)
        GH_TOKEN: 'fixture-token'
        GH_ARGS_LOG: $args_log
        GH_ASSET_URL: 'https://example.invalid/untrusted-response'
    } {
        ^$nu_exe $script owner/repository $valid --upload | complete
    }
    assert ($rejected_url.exit_code != 0) 'upload must reject a non-GitHub attachment URL'
    assert (not ($rejected_url.stderr | str contains 'example.invalid')) 'errors must not echo an untrusted upload response'

    let malformed = with-env {
        PATH: ($env.PATH | prepend $fake_bin)
        GH_TOKEN: 'fixture-token'
        GH_ARGS_LOG: $args_log
        GH_INVALID_JSON: '1'
    } {
        ^$nu_exe $script owner/repository $valid --upload | complete
    }
    assert ($malformed.exit_code != 0) 'upload must reject malformed JSON'
    assert (not ($malformed.stderr | str contains 'sensitive-service-response')) 'parse errors must not retain the service response'

    null
} catch {|err|
    $err | to nuon
}

rm --recursive --force $work
if $failure != null {
    print $failure
    error make {msg: 'E2E media uploader test failed'}
}

print 'E2E media uploader tests passed'
