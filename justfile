mod website

[private]
default:
    @just --list

hawk:
    nu scripts/hawk.nu

build-all: build build-debug website::build

build:
    nix build .#dupin

build-debug:
    cargo build -p dupin

run: build
    open result/Applications/Dupin.app

package channel="nightly" identity="-":
    nu scripts/package-app.nu --channel {{ channel }} --identity "{{ identity }}"

snap: build-debug
    nu scripts/snap.nu

docs-images *args: build-debug
    nu scripts/docs-images.nu {{ args }}

docs-clips *args: build-debug
    nu scripts/docs-images.nu --kind clips {{ args }}

footprint: build-debug
    nu scripts/footprint.nu
