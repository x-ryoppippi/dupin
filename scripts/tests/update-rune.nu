use std/assert
use ../update-rune.nu [pinned-rev locked-revs hashed-rev with-hash]

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

    assert equal (vendor $OLD "sha256-old=" | hashed-rev) $OLD
    # An empty hash is one still to compute, so it counts as none.
    assert equal (vendor $OLD "" | hashed-rev) null
    assert equal (vendor $OLD "sha256-old=" | with-hash $NEW "sha256-new=") (vendor $NEW "sha256-new=")
    assert error { "{ }" | with-hash $NEW "sha256-new=" }
}
