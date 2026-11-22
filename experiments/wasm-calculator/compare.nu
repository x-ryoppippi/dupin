#!/usr/bin/env nu

def --wrapped checked [...args: string]: nothing -> record {
    let result = ^$args.0 ...($args | skip 1) | complete
    if $result.exit_code != 0 {
        error make {msg: $"($args.0) failed:\n($result.stderr)"}
    }
    $result
}

def main [guest: path, --prepare-only]: nothing -> record {
    if $nu.os-info.name != 'macos' {
        error make {msg: 'This comparison uses macOS time -l byte counters.'}
    }
    let work = mktemp --directory | path expand
    let source = $work | path join source
    let files = [
        Cargo.toml
        Cargo.lock
        guest/Cargo.toml
        guest/src/lib.rs
        host/Cargo.toml
        host/src/main.rs
        compiled-host/Cargo.toml
        compiled-host/src/main.rs
    ]
    let sources = $files | each {|file|
        let destination = $source | path join $file
        mkdir ($destination | path dirname)
        cp ($env.FILE_PWD | path join $file) $destination
        {path: $file, sha256: (open --raw $destination | hash sha256)}
    }
    let module = $work | path join calculator.wasm
    cp ($guest | path expand) $module
    let script = $work | path join compare.nu
    cp ($env.FILE_PWD | path join compare.nu) $script
    let root = $env.FILE_PWD | path dirname | path dirname
    let lock = open --raw ($root | path join flake.lock) | from json
    let root_node = $lock.nodes | get $lock.root
    let metadata = {
        git_commit: (checked git -C $root rev-parse HEAD | get stdout | str trim)
        clean_tree: (
            checked git -C $root status --porcelain --untracked-files=normal
            | get stdout
            | str trim
            | is-empty
        )
        nixpkgs: ($lock.nodes | get $root_node.inputs.nixpkgs | get locked)
        system: $nu.os-info
        toolchain: (checked rustc --version --verbose | get stdout | str trim)
        macos: (checked /usr/bin/sw_vers | get stdout | str trim)
        cpu: (
            checked /usr/sbin/sysctl -n machdep.cpu.brand_string
            | get stdout
            | str trim
        )
        guest_bytes: (
            ls $module
            | first
            | get size
            | into int
        )
        guest_sha256: (open --raw $module | hash sha256)
        script_sha256: (open --raw $script | hash sha256)
        sources: $sources
        release_profile: 'fat LTO; one codegen unit; stripped; separate experimental workspace'
        compiled_module: 'single-threaded Cranelift compilation; no precompiled module'
        fuel: '20M per query in each runtime; fuel units are runtime-specific'
        memory: '16 MiB guest linear memory; host/runtime memory is not capped'
        signing: 'unsigned benchmark executables; no app signing or entitlement changes'
    }
    if $prepare_only {
        return {work: $work, metadata: $metadata, prepared: true}
    }
    let target = $work | path join target
    print --stderr $"Building isolated hosts in ($target)…"
    let runtimes = [
        {label: wasmi, binary: dupin-wasm-calculator-bench}
        {label: wasmtime, binary: dupin-compiled-calculator-bench}
    ]
    $runtimes | each {|runtime|
        let build = checked cargo build --locked --release --manifest-path (
            $source | path join Cargo.toml
        ) --target-dir $target -p $runtime.binary
        $build.stderr | save ($work | path join $"($runtime.label)-build.log")
        cp ($target | path join release $runtime.binary) ($work | path join $runtime.binary)
    } | ignore
    let runs = $runtimes | each {|runtime|
        let binary = $work | path join $runtime.binary
        let run = checked /usr/bin/time -l $binary $module
        let rss = $run.stderr | parse -r '(?<bytes>\d+)\s+maximum resident set size'
        let footprint = $run.stderr | parse -r '(?<bytes>\d+)\s+peak memory footprint'
        {
            runtime: $runtime.label
            executable_bytes: (ls $binary | first | get size | into int)
            executable_sha256: (open --raw $binary | hash sha256)
            maximum_resident_bytes: ($rss | first | get bytes | into int)
            peak_footprint_bytes: ($footprint | first | get bytes | into int)
            process_statistics: $run.stderr
            measurements: ($run.stdout | from json)
        }
    }
    let expected = $runs.0.measurements.queries | select query result
    if ($runs.1.measurements.queries | select query result) != $expected {
        error make {msg: 'The runtimes did not evaluate the same queries and results.'}
    }
    let result = {metadata: $metadata, runs: $runs}
    $result | to json | save ($work | path join results.json)
    {work: $work, result: $result}
}
