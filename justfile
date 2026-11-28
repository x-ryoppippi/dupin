mod telemetry
mod website

set lazy
set positional-arguments

# The flake checks that need the Rust builds: `ci-build` builds them, `ci-check` the rest.
rust_checks := "lint-and-test hawk dupin bundled-extension-scopes"
system := `nix eval --impure --raw --expr builtins.currentSystem`

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

render: build-debug
    nu scripts/render.nu

docs-images *args: build-debug
    nu scripts/docs-images.nu "$@"

docs-clips *args: build-debug
    nu scripts/docs-images.nu --kind clips "$@"

docs-glass *args: build
    nu scripts/docs-glass.nu "$@"

footprint: build-debug
    nu scripts/footprint.nu

# Every flake check except the Rust builds, so a new quick check runs here
ci-check:
    #!/usr/bin/env bash
    set -euo pipefail
    checks=()
    for name in $(nix eval --raw .#checks.{{ system }} --apply 'checks: toString (builtins.attrNames checks)'); do
      case " {{ rust_checks }} " in
        *" $name "*) ;;
        *) checks+=(".#checks.{{ system }}.$name") ;;
      esac
    done
    nix build --no-link "${checks[@]}"

# Clippy, tests, Hawk, the release app bundle and the scopes its bundled extensions declare
ci-build:
    nix build --no-link {{ prepend('.#checks.' + system + '.', rust_checks) }}

# The release bundle as a tarball, which keeps the executable bits upload-artifact drops
bundle-archive out="target/Dupin.app.tar.gz": build
    tar -czf {{ quote(out) }} -C result/Applications Dupin.app

# Measure result/Applications/Dupin.app (scripts/size.nu measure)
size-measure *args:
    nu --no-config-file scripts/size.nu measure result/Applications/Dupin.app "$@"

# Compare a size measurement with main's (scripts/size.nu report)
size-report *args:
    nu --no-config-file scripts/size.nu report "$@"

# The tests that start the extension host, natively, since macOS refuses its sandbox inside Nix's
extension-host-test:
    #!/usr/bin/env bash
    set -euo pipefail
    packages=()
    while IFS= read -r crate; do
      packages+=("--package=$crate")
    done < <(git grep -l 'skip_inside_a_sandbox!' -- 'crates/*.rs' | cut -d/ -f2 | sort -u)
    test "${#packages[@]}" -gt 0
    echo "Testing ${packages[*]}"
    DUPIN_REQUIRE_EXTENSION_HOST=1 cargo test --locked "${packages[@]}"

# Compile the workspace's tests without running them
warm-tests:
    cargo test --workspace --no-run --locked

# Build the latency binary of the flake at `flake`, another commit's checkout to record its native baseline
latency-build flake=".":
    nix build "$1#dupin-latency" --out-link result-latency

# Measure keypress latency with `latency-build`'s binary (scripts/latency.nu run)
latency-measure *args:
    nu scripts/latency.nu run --bin result-latency/bin/dupin "$@"

# Download the results of main's build to compare with (scripts/latency.nu fetch-main)
latency-fetch-main *args:
    nu --no-config-file scripts/latency.nu fetch-main "$@"

# Compare latency results with main's (scripts/latency.nu report)
latency-report *args:
    nu --no-config-file scripts/latency.nu report "$@"

# Hold the bundled extensions to native latency (scripts/latency.nu parity)
latency-parity *args:
    nu scripts/latency.nu parity "$@"

search-quality-build:
    nix build .#dupin-search-quality --out-link result-search-quality

# Run the search-quality cases with `search-quality-build`'s binary (scripts/search-quality.nu run)
search-quality-measure *args:
    nu --no-config-file scripts/search-quality.nu run --bin result-search-quality/bin/dupin-search-quality "$@"

# Compare search-quality results with main's (scripts/search-quality.nu report)
search-quality-report *args:
    nu --no-config-file scripts/search-quality.nu report "$@"

# Drive a built Dupin.app with agent-desktop (scripts/e2e.nu run)
e2e app *args:
    scripts/e2e.nu run "$@"

# Whether the nightly at `repo` was already built from `head` (scripts/nightly.nu)
nightly-check repo head:
    nu --no-config-file scripts/nightly.nu {{ quote(repo) }} {{ quote(head) }}

# Sign, notarize and package a release; needs the Developer ID identity and NOTARY_KEY_*
package-release channel:
    nu scripts/package-app.nu --channel {{ quote(channel) }} --notarize

formatter:
    nix build --no-link .#formatter.{{ system }}

fmt:
    nix fmt

# Bring Cargo.lock and nix/cargo-vendor.nix in step with the Rune rev Cargo.toml pins
update-rune:
    nu scripts/update-rune.nu

# Create or update the live rulesets to match .github/rulesets (scripts/ruleset.nu apply)
ruleset-apply *args:
    nu --no-config-file scripts/ruleset.nu apply "$@"

# Show how the live rulesets differ from .github/rulesets (scripts/ruleset.nu diff)
ruleset-diff *args:
    nu --no-config-file scripts/ruleset.nu diff "$@"
