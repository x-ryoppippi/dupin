#!/usr/bin/env nu

use ../../scripts/unlink-store.nu

const BASELINE = 'e58a51417f0062c48a125fdb71797b2ee6e54470'
const O3_PROFILE = "\n[profile.release.package.rquickjs-sys]\nopt-level = 3\n"

def --wrapped checked [...args: string]: nothing -> string {
    let result = run-external ...$args | complete
    if $result.exit_code != 0 {
        error make {msg: $"($args.0) failed:\n($result.stderr)"}
    }
    $result.stdout | str trim
}

def bytes [file: path]: nothing -> int {
    ls --all $file | get 0.size | into int
}

def app-bytes [app: path]: nothing -> int {
    glob $"($app)/**/*" --no-dir --no-symlink
    | each {|file| bytes $file }
    | math sum
}

def package [label: string, source: path, artifacts: path]: nothing -> record {
    let app = $artifacts | path join $"($label).app"
    let archive = $artifacts | path join $"($label).zip"
    checked /usr/bin/ditto $source $app | ignore
    checked chmod -R u+w $app | ignore
    unlink-store ($app | path join Contents MacOS dupin)
    checked /usr/bin/codesign --force --options runtime --sign - $app | ignore
    checked /usr/bin/codesign --verify --strict $app | ignore
    checked /usr/bin/ditto -c -k --keepParent $app $archive | ignore
    {
        label: $label
        raw_executable_bytes: (bytes ($source | path join Contents MacOS dupin))
        signed_executable_bytes: (bytes ($app | path join Contents MacOS dupin))
        signed_app_file_bytes: (app-bytes $app)
        zip_bytes: (bytes $archive)
        app: ($app | path relative-to ($artifacts | path dirname))
    }
}

def build [label: string, source: path, work: path]: nothing -> string {
    print --stderr $"Building ($label)…"
    (checked
        nix
        build
        $"path:($source)#dupin"
        --out-link
        ($work | path join $"($label)-result")
        --print-out-paths
    )
}

def size [work: path, root: path]: nothing -> record {
    let snapshots = $work | path join snapshots
    let tar = $work | path join source.tar
    let labels = [baseline js-oz js-o3]
    checked git -C $root archive --format=tar --output $tar $BASELINE | ignore
    for label in $labels {
        let source = $snapshots | path join $label
        mkdir $source
        checked tar -xf $tar -C $source | ignore
        if $label != baseline {
            (checked
                patch
                --batch
                --fuzz=0
                -p1
                -d
                $source
                --input
                ($env.FILE_PWD | path join probe.patch)
            ) | ignore
        }
    }
    $O3_PROFILE | save --append ($snapshots | path join js-o3 Cargo.toml)
    let lock = open --raw ($snapshots | path join baseline flake.lock) | from json
    let root_node = $lock.nodes | get $lock.root
    let nixpkgs = $lock.nodes | get $root_node.inputs.nixpkgs | get locked
    # A later build may trigger GC, so every package output keeps a root in the run directory.
    let outputs = $labels | each {|label|
        {label: $label, out: (build $label ($snapshots | path join $label) $work)}
    }
    let artifacts = $work | path join artifacts
    mkdir $artifacts
    let measurements = $outputs | each {|item|
        package $item.label ($item.out | path join Applications Dupin.app) $artifacts
    }
    let probes = $labels | skip 1 | each {|label|
        let executable = $artifacts | path join $"($label).app" Contents MacOS dupin
        let output = checked $executable --dupin-js-runtime-probe
        if not ($output | str contains 'js_runtime=ok title="2 words"') {
            error make {msg: $"unexpected ($label) probe output: ($output)"}
        }
        {label: $label, output: $output}
    }
    let baseline = $measurements | where label == baseline | first
    {
        nixpkgs: $nixpkgs
        toolchain: (
            checked nix shell --inputs-from ($snapshots | path join baseline) nixpkgs#rustc --command rustc --version --verbose
        )
        nix_outputs: (
            $outputs
            | reduce --fold {} {|item, acc| $acc | insert $item.label $item.out }
        )
        measurements: ($measurements | each {|item|
            $item | merge {
                signed_app_increase_bytes: ($item.signed_app_file_bytes - $baseline.signed_app_file_bytes)
                zip_increase_bytes: ($item.zip_bytes - $baseline.zip_bytes)
            }
        })
        probes: $probes
    }
}

def bench [work: path, samples: int, cold_runs: int]: nothing -> list<record> {
    let manifest = $env.FILE_PWD | path join bench Cargo.toml
    [
        {
            label: c-oz
            config: []
        }
        {
            label: c-o3
            config: [--config 'profile.release.package.rquickjs-sys.opt-level=3']
        }
    ] | each {|level|
        let target = $work | path join bench-target $level.label
        print --stderr $"Building the ($level.label) bench…"
        (checked
            cargo
            build
            --release
            --locked
            --manifest-path
            $manifest
            --target-dir
            $target
            ...$level.config
        ) | ignore
        let executable = $target | path join release dupin-js-runtime-bench
        let load_before = checked /usr/sbin/sysctl -n vm.loadavg
        let result = (checked
            $executable
            all
            --label
            $level.label
            --samples
            ($samples | into string)
            --cold-runs
            ($cold_runs | into string)
        ) | from json
        $result | merge {
            executable_bytes: (bytes $executable)
            load_average_before: $load_before
            load_average_after: (checked /usr/sbin/sysctl -n vm.loadavg)
        }
    }
}

def main [
    --prepare-only # Validate the source snapshots and patch without building.
    --skip-size # Only run the standalone bench.
    --skip-bench # Only measure the signed app.
    --samples: int = 2000 # Warm calls per workload.
    --cold-runs: int = 200 # Fresh processes per cold mode.
]: nothing -> record {
    let root = $env.FILE_PWD | path dirname | path dirname
    let system = checked nix eval --impure --raw --expr builtins.currentSystem
    if $system != 'aarch64-darwin' {
        error make {msg: 'This experiment requires an Apple silicon Mac.'}
    }
    let work = mktemp --directory | path expand
    let metadata = {
        baseline_commit: $BASELINE
        system: $system
        runtime: 'rquickjs 0.14.0 (QuickJS-ng 0.16.2); std only, without rust-alloc or allocator'
        limits: '16 MiB heap, 512 KiB stack, 100 ms interrupt deadline, 10,000 promise jobs'
        release_profile: 'fat LTO; one codegen unit; package.nix stripping; Rust opt-level z'
        signing: 'ad-hoc; hardened runtime; no additional entitlements; no notary submission'
        archive: 'ditto ZIP, as used by the updater; no DMG measurement'
        patch_sha256: (open --raw ($env.FILE_PWD | path join probe.patch) | hash sha256)
        script_sha256: (open --raw ($env.FILE_PWD | path join measure.nu) | hash sha256)
        bench_lock_sha256: (open --raw ($env.FILE_PWD | path join bench Cargo.lock) | hash sha256)
        unlink_store_sha256: (
            open --raw ($root | path join scripts unlink-store.nu) | hash sha256
        )
        macos: (checked /usr/bin/sw_vers)
        hardware: (checked /usr/sbin/sysctl -n machdep.cpu.brand_string)
    }
    if $prepare_only {
        let snapshot = $work | path join prepared
        mkdir $snapshot
        (checked
            git
            -C
            $root
            archive
            --format=tar
            --output
            ($work | path join source.tar)
            $BASELINE
        ) | ignore
        checked tar -xf ($work | path join source.tar) -C $snapshot | ignore
        (checked
            patch --batch --fuzz=0 -p1 -d $snapshot --input ($env.FILE_PWD | path join probe.patch)
        ) | ignore
        return {work: $work, metadata: $metadata, prepared: true}
    }
    let size = if $skip_size { null } else { size $work $root }
    let bench = if $skip_bench {
        null
    } else {
        {
            host_rustc: (checked rustc --version --verbose)
            host_cc: (checked cc --version)
            levels: (bench $work $samples $cold_runs)
        }
    }
    let result = {metadata: $metadata, size: $size, bench: $bench}
    $result | to json --tabs 1 | save ($work | path join results.json)
    {work: $work, result: $result}
}
