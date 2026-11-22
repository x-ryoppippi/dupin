#!/usr/bin/env nu

use ../../scripts/unlink-store.nu

const BASELINE = 'ac3991952f90435ea207e7c0bd9f876dec371028'

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
        app: $app
    }
}

def main [
    guest: path # A built bare calculator guest with the experiment's private ABI.
    --prepare-only # Validate the source snapshots and patch without building.
]: nothing -> record {
    let root = $env.FILE_PWD | path dirname | path dirname
    let system = checked nix eval --impure --raw --expr builtins.currentSystem
    if $system != 'aarch64-darwin' {
        error make {msg: 'This experiment requires an Apple silicon Mac.'}
    }
    let guest = $guest | path expand --strict
    let work = mktemp --directory | path expand
    let baseline = $work | path join baseline
    let variant = $work | path join variant
    let artifacts = $work | path join artifacts
    let tar = $work | path join source.tar
    mkdir $baseline $variant $artifacts
    checked git -C $root archive --format=tar --output $tar $BASELINE | ignore
    [$baseline $variant] | each {|source|
        checked tar -xf $tar -C $source | ignore
    } | ignore
    (checked
        patch
        --batch
        --fuzz=0
        -p1
        -d
        $variant
        --input
        ($env.FILE_PWD | path join probe.patch)
    ) | ignore
    let plugins = $variant | path join crates dupin assets plugins
    mkdir $plugins
    let copied_guest = $plugins | path join fend_calculator.wasm
    cp $guest $copied_guest
    0x[0061736d010000000105016000017f03020100070801046576616c00000a06010400412a0b]
    | save ($plugins | path join minimal_eval.wasm)
    let lock = open --raw ($baseline | path join flake.lock) | from json
    let root_node = $lock.nodes | get $lock.root
    let nixpkgs = $lock.nodes | get $root_node.inputs.nixpkgs | get locked
    let metadata = {
        baseline_commit: $BASELINE
        system: $system
        runtime: 'wasmi 2.0.2; std, validate, auto-dispatch; fuel enabled; 16 MiB linear-memory limit'
        release_profile: 'fat LTO; one codegen unit; package.nix stripping'
        signing: 'ad-hoc; hardened runtime; no additional entitlements; no notary submission'
        archive: 'ditto ZIP, as used by the updater; no DMG measurement'
        nixpkgs: $nixpkgs
        guest_bytes: (bytes $copied_guest)
        guest_sha256: (open --raw $copied_guest | hash sha256)
        patch_sha256: (open --raw ($env.FILE_PWD | path join probe.patch) | hash sha256)
        script_sha256: (open --raw ($env.FILE_PWD | path join measure.nu) | hash sha256)
        unlink_store_sha256: (
            open --raw ($root | path join scripts unlink-store.nu) | hash sha256
        )
    }
    if $prepare_only {
        return {work: $work, metadata: $metadata, prepared: true}
    }
    let metadata = $metadata | insert toolchain (
        checked nix shell --inputs-from $baseline nixpkgs#rustc --command rustc --version --verbose
    )
    # A later build may trigger GC, so keep both package outputs rooted.
    print --stderr 'Building runtime and guest variants…'
    let variant_out = (checked
        nix
        build
        $"path:($variant)#dupin"
        --out-link
        ($work | path join variant-result)
        --print-out-paths
    )
    print --stderr 'Building the pinned baseline package…'
    let baseline_out = (checked
        nix
        build
        $"path:($baseline)#dupin"
        --out-link
        ($work | path join baseline-result)
        --print-out-paths
    )
    let metadata = $metadata
    | insert nix_outputs {baseline: $baseline_out, variant: $variant_out}
    | insert macos (checked /usr/bin/sw_vers)
    let measurements = [
        {
            label: baseline
            source: ($baseline_out | path join Applications Dupin.app)
        }
        {
            label: wasmi-runtime
            source: ($variant_out | path join Applications Dupin-wasmi-runtime.app)
        }
        {
            label: wasmi-minimal
            source: ($variant_out | path join Applications Dupin-wasmi-minimal.app)
        }
        {
            label: wasmi-fend
            source: ($variant_out | path join Applications Dupin-wasmi-fend.app)
        }
    ] | each {|item| package $item.label $item.source $artifacts }
    let probes = [
        {
            label: wasmi-runtime
            flag: --dupin-wasmi-runtime-probe
            expected: 'wasmi_runtime=ok'
        }
        {
            label: wasmi-minimal
            flag: --dupin-wasmi-minimal-probe
            expected: 'minimal_guest_result=42'
        }
        {
            label: wasmi-fend
            flag: --dupin-wasmi-fend-probe
            expected: '347.9678676529 mi'
        }
    ] | each {|probe|
        let executable = $artifacts | path join $"($probe.label).app" Contents MacOS dupin
        let output = checked $executable $probe.flag
        if not ($output | str contains $probe.expected) {
            error make {msg: $"unexpected ($probe.label) probe output: ($output)"}
        }
        {label: $probe.label, output: $output}
    }
    let result = {metadata: $metadata, measurements: $measurements, probes: $probes}
    $result | to json | save ($work | path join results.json)
    {work: $work, result: $result}
}
