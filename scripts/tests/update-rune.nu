use std/assert
use ../update-rune.nu [
    pinned-rev
    patched-crates
    locked-revs
    in-step
    unapplied-patches
    unlock-specs
    hashed-rev
    with-hash
]

const OLD = "86b903de7fef9c6b683e05dc23440de4ebbc4347"
const NEW = "fcef33a2f60db30a7a872844464f84ff26f3fc75"
const RUNE = "https://github.com/anomalyco/rune"

def manifest [dependency: string, patch: string]: nothing -> record {
    {
        workspace: {
            dependencies: {
                rune: {
                    git: $RUNE
                    rev: $dependency
                    features: [svg]
                }
                serde: "1"
                dupin-core: {path: "crates/dupin-core"}
            }
        }
        patch: {
            crates-io: {
                winit: {git: $RUNE, rev: $patch}
                naga: {git: $RUNE, rev: $patch}
                objc2: {path: "vendor/objc2"}
            }
        }
    }
}

def vendor [rev: string, hash: string]: nothing -> string {
    $'  outputHashes = {
    "git+($RUNE)?rev=($rev)#($rev)" =
      "($hash)";
  };
'
}

def main [] {
    assert equal (manifest $NEW $NEW | pinned-rev) $NEW
    assert error { manifest $OLD $NEW | pinned-rev }
    assert error { {workspace: {dependencies: {serde: "1"}}} | pinned-rev }

    let lock = {
        package: [
            {
                name: serde
                source: "registry+https://github.com/rust-lang/crates.io-index"
            }
            {name: dupin}
            {
                name: rune
                source: $"git+($RUNE)?rev=($OLD)#($OLD)"
            }
            {
                name: winit
                source: $"git+($RUNE)?rev=($OLD)#($OLD)"
            }
        ]
    }
    assert equal ($lock | locked-revs) [$OLD]
    assert equal (manifest $NEW $NEW | patched-crates) [winit naga]

    let rune = $"git+($RUNE)?rev=($NEW)#($NEW)"
    let crates_io = "registry+https://github.com/rust-lang/crates.io-index"
    let relocked = {
        package: [
            {name: rune, version: "0.0.1", source: $rune}
            {name: winit, version: "0.30.13", source: $rune}
            {name: naga, version: "30.0.2", source: $crates_io}
        ]
        patch: {
            unused: [
                {name: naga, version: "30.0.1", source: $rune}
            ]
        }
    }
    # Moving Rune's own packages leaves a newly patched crate locked from crates.io.
    assert equal ($relocked | locked-revs) [$NEW]
    assert not ($relocked | in-step $NEW [winit naga])
    assert equal ($relocked | unapplied-patches [winit naga]) [naga]
    assert equal ($relocked | unlock-specs [winit naga]) ["naga@30.0.2"]

    let naga = {name: naga, version: "30.0.1", source: $rune}
    let patched = $relocked
    | update package { each {|package| if $package.name == naga { $naga } else { $package } } }
    | reject patch
    assert ($patched | in-step $NEW [winit naga])
    assert equal ($patched | unlock-specs [winit naga]) []
    assert not ($patched | in-step $OLD [winit naga])

    assert equal (vendor $OLD "sha256-old=" | hashed-rev) $OLD
    # An empty hash is one still to compute, so it counts as none.
    assert equal (vendor $OLD "" | hashed-rev) null
    assert equal (vendor $OLD "sha256-old=" | with-hash $NEW "sha256-new=") (vendor $NEW "sha256-new=")
    assert error { "{ }" | with-hash $NEW "sha256-new=" }
}
