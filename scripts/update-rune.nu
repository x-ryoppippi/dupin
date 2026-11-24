#!/usr/bin/env nu

# Bring Cargo.lock and nix/cargo-vendor.nix in step with the Rune rev Cargo.toml pins.
#
# Cargo.toml pins every Rune crate at one rev, but Cargo.lock and Rune's source hash in
# nix/cargo-vendor.nix only follow when someone refreshes them. autofix.ci runs this on every
# pull request that touches either file, so a Renovate PR that only rewrites the revs comes
# back buildable, and so does one where someone adds a patch for a crate Rune newly patches.
# When the lock and the hash already match the pinned rev, it changes nothing and reaches no
# network.

const RUNE = "https://github.com/anomalyco/rune"
const VENDOR = "nix/cargo-vendor.nix"

def main []: nothing -> nothing {
    cd ($env.FILE_PWD | path dirname)
    let manifest = open Cargo.toml
    let rev = $manifest | pinned-rev
    let patched = $manifest | patched-crates

    let lock = open --raw Cargo.lock | from toml
    if not ($lock | in-step $rev $patched) {
        print $"Refreshing Cargo.lock's Rune packages to ($rev)"
        let specs = [
            "rune"
            ...($lock | unlock-specs $patched)
        ]
        ^cargo update ...($specs | each {|spec| [--package $spec] } | flatten)

        let lock = open --raw Cargo.lock | from toml
        let unapplied = $lock | unapplied-patches $patched
        if ($unapplied | is-not-empty) {
            let crates = $unapplied | str join ", "
            error make {
                msg: $"Cargo.toml patches ($crates) from Rune, but Cargo.lock takes it from elsewhere. A patch applies only to a version every requirement accepts, so match Dupin's own requirement to Rune's version, or drop a patch Rune no longer has."
            }
        }
        if ($lock | locked-revs) != [$rev] {
            error make {msg: $"Cargo.lock still has Rune packages away from ($rev)"}
        }
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

# The crates Cargo.toml patches from Rune.
export def patched-crates []: record -> list<string> {
    $in.patch?.crates-io?
    | default {}
    | transpose name dep
    | where {|row| ($row.dep | describe) =~ '^record' and $row.dep.git? == $RUNE }
    | get name
}

# Whether Cargo.lock takes every Rune package at `rev`, every crate in `patched` among them.
export def in-step [rev: string, patched: list<string>]: record -> bool {
    let lock = $in
    ($lock | locked-revs) == [$rev] and ($lock | unapplied-patches $patched | is-empty)
}

# The crates in `patched` Cargo.lock takes no package of from Rune. Cargo files such a patch
# under `[[patch.unused]]` and keeps the crate locked where it was.
export def unapplied-patches [patched: list<string>]: record -> list<string> {
    let from_rune = $in.package | where {|package| $package | from-rune } | get name
    $patched | where {|name| $name not-in $from_rune }
}

# Package specs for `cargo update` that unlock every crate in `patched` Cargo.lock takes from
# elsewhere. `cargo update --package rune` leaves those locked, so a newly patched crate never
# moves.
export def unlock-specs [patched: list<string>]: record -> list<string> {
    let lock = $in
    let unapplied = $lock | unapplied-patches $patched
    $lock.package
    | where {|package| $package.name in $unapplied }
    | each {|package| $"($package.name)@($package.version)" }
}

def from-rune []: record -> bool {
    $in.source? | default "" | str starts-with $"git+($RUNE)?"
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
