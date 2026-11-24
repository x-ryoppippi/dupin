#!/usr/bin/env nu

# Bring Cargo.lock and nix/cargo-vendor.nix in step with the Rune rev Cargo.toml pins.
#
# Cargo.toml pins every Rune crate at one rev, but Cargo.lock and Rune's source hash in
# nix/cargo-vendor.nix only follow when someone refreshes them. autofix.ci runs this on every
# pull request that touches either file, so a Renovate PR that only rewrites the revs comes
# back buildable. When the lock and the hash already match the pinned rev, it changes nothing
# and reaches no network.

const RUNE = "https://github.com/anomalyco/rune"
const VENDOR = "nix/cargo-vendor.nix"

def main []: nothing -> nothing {
    cd ($env.FILE_PWD | path dirname)
    let rev = open Cargo.toml | pinned-rev

    if (open --raw Cargo.lock | from toml | locked-revs) != [$rev] {
        print $"Refreshing Cargo.lock's Rune packages to ($rev)"
        ^cargo update --package rune
    }

    let vendor = open --raw $VENDOR
    if ($vendor | hashed-rev) != $rev {
        print $"Hashing Rune ($rev) for ($VENDOR)"
        $vendor | with-hash $rev (source-hash $rev) | save --force $VENDOR
    }
}

# The one rev Cargo.toml pins Rune's crates at, in its dependencies and its patches.
export def pinned-rev []: record -> string {
    let manifest = $in
    let revs = [
        ($manifest.workspace?.dependencies? | default {})
        ($manifest.patch?.crates-io? | default {})
    ]
    | each { values }
    | flatten
    | where {|dep| ($dep | describe) =~ '^record' and $dep.git? == $RUNE }
    | each {|dep| $dep.rev? }
    | uniq

    match $revs {
        [] => { error make {msg: $"Cargo.toml has no dependency on ($RUNE)"} }
        [$rev] if $rev != null => { $rev }
        _ => {
            error make {
                msg: $"Cargo.toml must pin every Rune crate at one rev, found: ($revs | to nuon)"
            }
        }
    }
}

# The revs Cargo.lock has Rune's packages at.
export def locked-revs []: record -> list<string> {
    $in.package
    | get --optional source
    | compact
    | where {|source| $source | str starts-with $"git+($RUNE)?" }
    | each { parse --regex '#(?<rev>[0-9a-f]+)$' | get 0.rev }
    | uniq
}

# The rev nix/cargo-vendor.nix holds Rune's hash for, or null without one.
export def hashed-rev []: string -> any {
    $in
    | parse --regex ((entry-pattern '(?<rev>[0-9a-f]+)') + '"sha256-[^"]+"')
    | get --optional 0.rev
}

# nix/cargo-vendor.nix with Rune's entry in outputHashes keyed to `rev` and holding `hash`.
export def with-hash [rev: string, hash: string]: string -> string {
    let vendor = $in
    let entry = $'"git+($RUNE)?rev=($rev)#($rev)" =(char newline)      "($hash)";'
    let pattern = (entry-pattern '[0-9a-f]+') + '"[^"]*";'
    if ($vendor | parse --regex $pattern | length) != 1 {
        error make {msg: $"($VENDOR) must hold exactly one outputHashes entry for ($RUNE)"}
    }
    $vendor | str replace --regex $pattern $entry
}

# Matches an outputHashes key for Rune at `rev` (a regex) and the `=` after it.
def entry-pattern [rev: string]: nothing -> string {
    '"git\+' + ($RUNE | escape-regex) + '\?rev=' + $rev + '#[0-9a-f]+"\s*=\s*'
}

def escape-regex []: string -> string {
    $in | str replace --all --regex '([.?*+^$()\[\]{}|\\])' '\$1'
}

# The hash of Rune's source at `rev`, as crane fetches it for nix/cargo-vendor.nix: building
# crane's own fetch with an empty hash makes Nix report the one it got.
def source-hash [rev: string]: nothing -> string {
    let expr = $'
        let
          flake = builtins.getFlake "(pwd)";
          pkgs = flake.inputs.nixpkgs.legacyPackages.${builtins.currentSystem};
          craneLib = flake.inputs.crane.mkLib pkgs;
        in
        craneLib.downloadCargoPackageFromGit {
          git = "($RUNE)";
          rev = "($rev)";
          hash = "";
        }
    '
    let build = ^nix build --no-link --impure --expr $expr | complete
    let got = $build.stderr | parse --regex 'got:\s+(?<hash>sha256-[A-Za-z0-9+/=]+)' | get --optional 0.hash
    if $got == null {
        error make {msg: $"Nix reported no hash for Rune ($rev):\n($build.stderr)"}
    }
    $got
}
