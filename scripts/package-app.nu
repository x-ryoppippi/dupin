#!/usr/bin/env nu

# Turn the Nix-built Dupin.app into a release: give it the channel's version and update feed,
# sign it with a Developer ID, notarize and staple it, and write <out>/Dupin.zip (what the
# updater installs), <out>/Dupin.dmg (what people download by hand) and <out>/update.json for
# the GitHub release.
#
# Signing needs the Developer ID Application identity in a keychain. Notarizing needs an App
# Store Connect API key: NOTARY_KEY_PATH (the .p8 file), NOTARY_KEY_ID and NOTARY_ISSUER.
# Without `--notarize` the zip is signed but not notarized, for trying the bundle locally.
def main [
    --channel: string # "stable" or "nightly"
    --identity: string = "Developer ID Application" # A codesign identity, or "-" for ad-hoc
    --notarize
    --app: path # A built Dupin.app; built with `nix build .#dupin` when absent
    --out: path = "dist"
]: nothing -> record {
    let root = $env.FILE_PWD | path dirname
    cd $root
    let repo = "x-ryoppippi/dupin"
    let base = cat VERSION | str trim
    # Commits along main only grow, so the count orders builds across both channels.
    let build = ^git rev-list --count HEAD | str trim
    let release = match $channel {
        stable => {
            version: $base
            feed: $"https://github.com/($repo)/releases/latest/download/update.json"
            download: $"https://github.com/($repo)/releases/download/v($base)/Dupin.zip"
        }
        nightly => {
            version: $"($base)-nightly.($build)+(^git rev-parse --short HEAD | str trim)"
            feed: $"https://github.com/($repo)/releases/download/nightly/update.json"
            download: $"https://github.com/($repo)/releases/download/nightly/Dupin.zip"
        }
        _ => { error make {msg: $"unknown channel: ($channel)"} }
    }

    let built = if $app != null { $app } else {
        ^nix build .#dupin --no-link --print-out-paths | str trim | path join Applications/Dupin.app
    }
    rm --recursive --force $out
    mkdir $out
    let bundle = $out | path join Dupin.app
    # The store copy is read-only; the release copy is edited and signed.
    ^/usr/bin/ditto $built $bundle
    ^chmod -R u+w $bundle

    unlink-store ($bundle | path join Contents/MacOS/dupin)

    let plist = $bundle | path join Contents/Info.plist
    ^/usr/bin/plutil -replace CFBundleShortVersionString -string $release.version $plist
    ^/usr/bin/plutil -replace CFBundleVersion -string $build $plist
    ^/usr/bin/plutil -insert DupinUpdateFeed -string $release.feed $plist
    if $channel == nightly {
        ^/usr/bin/plutil -replace CFBundleIconFile -string dupin-nightly $plist
    }

    ^/usr/bin/codesign --force --options runtime --timestamp --sign $identity $bundle
    ^/usr/bin/codesign --verify --strict --verbose=2 $bundle

    let zip = $out | path join Dupin.zip
    let key = if $notarize { notary-key } else { null }
    if $key != null {
        ^/usr/bin/ditto -c -k --keepParent $bundle $zip
        notarize $zip $key
        ^/usr/bin/xcrun stapler staple $bundle
        ^/usr/sbin/spctl --assess --type execute --verbose=2 $bundle
        rm $zip
    }
    ^/usr/bin/ditto -c -k --keepParent $bundle $zip

    # Opening a downloaded zip leaves Dupin.app in Downloads, where App Translocation runs it
    # from a read-only copy that can't update itself; a disk image asks to drag it to
    # Applications first.
    let dmg = $out | path join Dupin.dmg
    ^nix shell --inputs-from $root nixpkgs#python3Packages.dmgbuild --command dmgbuild -s packaging/dmg/settings.py -D $"app=($bundle)" -D $"background=(dmg-background)" Dupin $dmg
    ^/usr/bin/codesign --force --timestamp --sign $identity $dmg
    if $key != null {
        notarize $dmg $key
        ^/usr/bin/xcrun stapler staple $dmg
        ^/usr/sbin/spctl --assess --type open --context context:primary-signature --verbose=2 $dmg
    }

    let feed = {
        version: $release.version
        build: ($build | into int)
        url: $release.download
        sha256: (open --raw $zip | hash sha256)
    }
    $feed | to json | save ($out | path join update.json)
    $feed
}

# The disk image's window background, at 1x and 2x in one TIFF so Finder picks the sharp one.
def dmg-background []: nothing -> string {
    let rendered = ^nix build --no-link --print-out-paths .#dmg-background | str trim
    let tiff = mktemp --suffix .tiff
    ^/usr/bin/tiffutil -cathidpicheck ($rendered | path join background.png) ($rendered | path join background@2x.png) -out $tiff
    $tiff
}

def notary-key []: nothing -> record {
    let key = {
        path: ($env.NOTARY_KEY_PATH? | default "")
        id: ($env.NOTARY_KEY_ID? | default "")
        issuer: ($env.NOTARY_ISSUER? | default "")
    }
    if ($key | values | any { is-empty }) {
        error make {
            msg: "notarizing needs NOTARY_KEY_PATH, NOTARY_KEY_ID and NOTARY_ISSUER"
        }
    }
    $key
}

def notarize [file: path, key: record]: nothing -> nothing {
    ^/usr/bin/xcrun notarytool submit $file --key $key.path --key-id $key.id --issuer $key.issuer --wait
}

# Point the binary's /nix/store libraries at the copies macOS ships, so the bundle runs
# without Nix. A library macOS doesn't ship stops the release instead.
def unlink-store [binary: path]: nothing -> nothing {
    let system = [libiconv.2.dylib]
    let linked = ^/usr/bin/otool -L $binary
    | lines
    | skip 1
    | str trim
    | parse "{lib} (compatibility {rest}"
    | get lib
    | str trim
    | where { str starts-with /nix/store/ }
    for lib in $linked {
        let name = $lib | path basename
        if $name not-in $system {
            error make {msg: $"($binary) links ($lib), which macOS doesn't ship"}
        }
        ^/usr/bin/install_name_tool -change $lib $"/usr/lib/($name)" $binary
    }
}
