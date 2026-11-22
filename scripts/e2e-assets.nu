#!/usr/bin/env nu

const MAX_ASSETS = 50
const MAX_IMAGE_BYTES = 10_000_000
const MAX_VIDEO_BYTES = 100_000_000
const SAFE_NAME = '^[A-Za-z0-9][A-Za-z0-9._-]{0,119}\.(?i:png|jpe?g|gif|webp|mp4|mov|webm)\z'
const ASSET_URL = '^https://github\.com/user-attachments/assets/[A-Za-z0-9_-]{16,80}\z'

def media-details [extension: string]: nothing -> any {
    match $extension {
        '.png' => {kind: 'image', mime_type: 'image/png'}
        '.jpg' => {kind: 'image', mime_type: 'image/jpeg'}
        '.jpeg' => {kind: 'image', mime_type: 'image/jpeg'}
        '.gif' => {kind: 'image', mime_type: 'image/gif'}
        '.webp' => {kind: 'image', mime_type: 'image/webp'}
        '.mp4' => {kind: 'video', mime_type: 'video/mp4'}
        '.mov' => {kind: 'video', mime_type: 'video/quicktime'}
        '.webm' => {kind: 'video', mime_type: 'video/webm'}
        _ => { null }
    }
}

def valid-signature [data: binary, extension: string]: nothing -> bool {
    match $extension {
        '.png' => {
            $data | bytes starts-with 0x[89 50 4E 47 0D 0A 1A 0A]
        }
        '.jpg' | '.jpeg' => {
            $data | bytes starts-with 0x[FF D8 FF]
        }
        '.gif' => { ($data | bytes starts-with ('GIF87a' | into binary)) or ($data | bytes starts-with ('GIF89a' | into binary)) }
        '.webp' => {
            (
                (($data | bytes length) >= 12) and ($data | bytes starts-with 0x[52 49 46 46]) and (($data | bytes at 8..11) == 0x[57 45 42 50])
            )
        }
        '.mp4' | '.mov' => {
            (
                (($data | bytes length) >= 12) and (($data | bytes at 4..7) == ('ftyp' | into binary))
            )
        }
        '.webm' => {
            $data | bytes starts-with 0x[1A 45 DF A3]
        }
        _ => { false }
    }
}

export def inspect-assets [directory: path]: nothing -> table {
    let root = $directory | path expand --no-symlink
    if ($root | path type) != 'dir' {
        error make {msg: 'E2E media directory must be a real directory'}
    }

    let assets = ls --all --long $root
    | where type == 'file'
    | where {|entry| ($entry.name | path type) == 'file'}
    | each {|entry|
        let name = $entry.name | path basename
        let extension = $name | path parse | get extension | str lowercase
        let extension = $'.($extension)'
        let details = media-details $extension
        if $details == null {
            null
        } else {
            {
                path: ($entry.name | path expand --no-symlink)
                name: $name
                extension: $extension
                kind: $details.kind
                mime_type: $details.mime_type
                size: ($entry.size | into int)
            }
        }
    }
    | where $it != null
    | sort-by name

    if ($assets | length) > $MAX_ASSETS {
        error make {msg: $'E2E media has more than ($MAX_ASSETS) attachable files'}
    }

    $assets | each {|asset|
        if not ($asset.name =~ $SAFE_NAME) {
            error make {msg: 'unsafe E2E media filename'}
        }
        if $asset.size <= 0 {
            error make {msg: $'E2E media file is empty: ($asset.name)'}
        }
        let max_bytes = if $asset.kind == 'image' { $MAX_IMAGE_BYTES } else { $MAX_VIDEO_BYTES }
        let max_size = if $asset.kind == 'image' { '10 MB' } else { '100 MB' }
        if $asset.size > $max_bytes {
            error make {msg: $'E2E ($asset.kind) file exceeds ($max_size): ($asset.name)'}
        }

        let data = open --raw $asset.path | into binary
        if not (valid-signature $data $asset.extension) {
            error make {msg: $'E2E media signature does not match ($asset.name)'}
        }

        $asset
    }
}

def repository-id [repository: string]: nothing -> int {
    let result = ^gh api --hostname github.com $'repos/($repository)' --jq '.id' | complete
    if $result.exit_code != 0 {
        error make {msg: 'could not resolve the GitHub repository ID'}
    }

    let id = try {
        $result.stdout | str trim | into int
    } catch {
        null
    }
    if $id == null or $id <= 0 {
        error make {msg: 'GitHub returned an invalid repository ID'}
    }
    $id
}

def upload-one [repo_id: int, asset: record]: nothing -> string {
    if ($asset.path | path type) != 'file' {
        error make {msg: $'E2E media file changed during upload: ($asset.name)'}
    }

    let result = ^gh api --hostname github.com 'https://uploads.github.com/user-attachments/assets' --method POST --input $asset.path --header 'Content-Type: application/octet-stream' --header 'Accept: application/vnd.github+json' --field $'name=($asset.name)' --field $'content_type=($asset.mime_type)' --field $'repository_id=($repo_id)' | complete

    if $result.exit_code != 0 {
        error make {msg: $'GitHub could not upload E2E media: ($asset.name)'}
    }

    let response = try {
        $result.stdout | from json
    } catch {
        null
    }
    if not (($response | describe) | str starts-with 'record') {
        error make {msg: $'GitHub returned an invalid upload response for ($asset.name)'}
    }
    let url = $response.url? | default ''
    if ($url | describe) != 'string' or not ($url =~ $ASSET_URL) {
        error make {msg: $'GitHub returned an unexpected attachment URL for ($asset.name)'}
    }
    $url
}

def upload-assets [repository: string, assets: table]: nothing -> table {
    if ($assets | is-empty) {
        return []
    }

    if ($env.GH_TOKEN? | default '' | is-empty) {
        error make {msg: 'upload requires GH_TOKEN'}
    }

    let id = repository-id $repository
    $assets | each {|asset|
        {
            name: $asset.name
            kind: $asset.kind
            mime_type: $asset.mime_type
            size: $asset.size
            url: (upload-one $id $asset)
        }
    }
}

export def main [repository: string, directory: path, --upload]: nothing -> string {
    if not ($repository =~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9][A-Za-z0-9_.-]{0,99}\z') {
        error make {msg: 'repository must be OWNER/REPOSITORY'}
    }

    let assets = inspect-assets $directory
    let result_assets = if $upload {
        upload-assets $repository $assets
    } else {
        $assets | each {|asset|
            {
                name: $asset.name
                kind: $asset.kind
                mime_type: $asset.mime_type
                size: $asset.size
                url: null
            }
        }
    }

    {repository: $repository, uploaded: $upload, assets: $result_assets} | to json --raw
}
