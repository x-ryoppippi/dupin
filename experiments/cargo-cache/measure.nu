def run-command [
    program: string
    args: list<string>
    environment: record
    hidden_environment: list<string>
] {
    hide-env --ignore-errors ...$hidden_environment
    with-env $environment {
    run-external $program ...$args | complete
  }
}

def require-success [label: string, result: record] {
    if $result.exit_code != 0 {
        error make {
            msg: $"($label) failed with exit code ($result.exit_code):\n($result.stderr)\n($result.stdout)"
        }
    }

    $result.stdout
}

def sccache-stats [sccache: path, environment: record, hidden_environment: list<string>] {
    let output = (run-command
        ($sccache | into string)
        ["--show-stats" "--stats-format=json"]
        $environment
        $hidden_environment
    )
    require-success "sccache --show-stats" $output | from json
}

def measure-build [
    sccache: path
    package: string
    name: string
    target: path
    wrapped: bool
    environment: record
    hidden_environment: list<string>
] {
    let target_dir = $target | path expand
    let command = [
        "build"
        "--offline"
        "--package" $package
        "--target-dir" ($target_dir | into string)
        "--message-format=json"
    ]
    let compiler = if $wrapped {
        $sccache | into string
    } else {
        ""
    }
    let child_environment = $environment | upsert RUSTC_WRAPPER $compiler

    let _ = require-success "sccache --zero-stats" (
    run-command ($sccache | into string) ["--zero-stats"] $environment $hidden_environment
  )

    print --stderr $"Measuring ($name): cargo ($command | str join ' ')"
    let measured = timeit --output {
    run-command "cargo" $command $child_environment $hidden_environment
  }
    let cargo_result = $measured.output
    let cargo_stdout = require-success $"cargo ($command | str join ' ')" $cargo_result
    let messages = $cargo_stdout | lines | where {|line| $line | str trim | is-not-empty} | each { from json }
    let artifacts = $messages
    | where reason == "compiler-artifact"
    | each {|artifact|
      {
        target: $artifact.target.name
        cargo_fresh: $artifact.fresh
      }
    }
    let cache = sccache-stats $sccache $environment $hidden_environment

    {
        name: $name
        command: (["cargo"] ++ $command)
        target_dir: ($target_dir | into string)
        rustc_wrapper: $compiler
        elapsed: $measured.time
        cargo_artifacts: $artifacts
        sccache: $cache
    }
}

def main [sccache: path, --package(-p): string = "dupin", --output(-o): path = "results.json"] {
    const script = path self
    let script_sha256 = open --raw $script | hash sha256
    let sccache = $sccache | path expand
    if not ($sccache | path exists) {
        error make {msg: $"sccache executable does not exist: ($sccache)"}
    }

    let output_path = $output | path expand
    let output_parent = $output_path | path dirname
    if not ($output_parent | path exists) {
        error make {msg: $"results directory does not exist: ($output_parent)"}
    }
    if ($output_path | path type) == "dir" {
        error make {msg: $"results path is a directory: ($output_path)"}
    }

    let temp_root = mktemp --directory --tmpdir-path "/tmp" "dupin-cargo-cache.XXXXXXXX" | path expand
    let cache_dir = $temp_root | path join "cache"
    let socket_path = $temp_root | path join "sccache.sock"
    let config_path = $temp_root | path join "sccache.toml"
    let config = {
        cache: {
            disk: {
                dir: ($cache_dir | into string)
                size: 2147483648
            }
        }
    } | to toml
    $config | save --force $config_path

    let inherited_sccache = $env
    | columns
    | where {|key| $key | str starts-with "SCCACHE_"}
    let hidden_environment = ($inherited_sccache ++ ["CARGO_INCREMENTAL"]) | uniq
    let sccache_environment = {
        SCCACHE_CONF: ($config_path | into string)
        SCCACHE_DIR: ($cache_dir | into string)
        SCCACHE_SERVER_UDS: ($socket_path | into string)
        SCCACHE_CACHE_SIZE: "2G"
        RUSTC_WORKSPACE_WRAPPER: ""
    }

    let version = require-success "sccache --version" (
    run-command ($sccache | into string) ["--version"] $sccache_environment $hidden_environment
  ) | str trim
    let cargo_version = require-success "cargo --version" (
    run-command "cargo" ["--version"] $sccache_environment $hidden_environment
  ) | str trim
    let rustc_version = require-success "rustc -vV" (
    run-command "rustc" ["-vV"] $sccache_environment $hidden_environment
  ) | str trim
    let git_commit = require-success "git rev-parse HEAD" (
    run-command "git" ["rev-parse" "HEAD"] {} $hidden_environment
  ) | str trim
    let working_tree_dirty = (require-success "git status --porcelain" (
    run-command "git" ["status" "--porcelain"] {} $hidden_environment
  ) | str trim) != ""

    let _ = require-success "sccache --start-server" (
    run-command ($sccache | into string) ["--start-server"] $sccache_environment $hidden_environment
  )
    let run = try {
        let initial_cases = [
            {
                name: "native_first_fresh_target"
                wrapped: false
                target: "native-a"
            }
            {
                name: "sccache_cold_fresh_target"
                wrapped: true
                target: "cached-a"
            }
            {
                name: "sccache_warm_fresh_target"
                wrapped: true
                target: "cached-b"
            }
            {
                name: "native_control_fresh_target"
                wrapped: false
                target: "native-b"
            }
            {
                name: "sccache_warm_reused_target"
                wrapped: true
                target: "cached-b"
            }
            {name: "native_reused_target", wrapped: false, target: "native-b"}
        ]
        let first_results = $initial_cases | each {|case|
        measure-build $sccache $package $case.name ($temp_root | path join $case.target) $case.wrapped $sccache_environment $hidden_environment
      }

        let cached_same_path = $temp_root | path join "cached-b"
        let native_same_path = $temp_root | path join "native-b"
        if ($cached_same_path | path exists) { rm --recursive --force $cached_same_path }
        if ($native_same_path | path exists) { rm --recursive --force $native_same_path }
        let same_path_cases = [
            {
                name: "sccache_warm_same_absolute_target_after_removal"
                wrapped: true
                target: ($cached_same_path | into string)
            }
            {
                name: "native_same_absolute_target_after_removal"
                wrapped: false
                target: ($native_same_path | into string)
            }
        ]
        let last_results = $same_path_cases | each {|case|
        measure-build $sccache $package $case.name $case.target $case.wrapped $sccache_environment $hidden_environment
      }

        {
            status: "ok"
            results: ($first_results ++ $last_results)
        }
    } catch {|error|
        {status: "failed", error: $error.rendered}
    } finally {
        let stopped = (run-command
                ($sccache | into string)
                ["--stop-server"]
                $sccache_environment
                $hidden_environment
            )
        if $stopped.exit_code != 0 {
            print --stderr $"Could not stop the isolated sccache server: ($stopped.stderr)"
        }
    }
    let data = {
        format_version: 1
        source_commit: $git_commit
        working_tree_dirty: $working_tree_dirty
        working_directory: (pwd | path expand | into string)
        package: $package
        sccache_executable: ($sccache | into string)
        sccache_version: $version
        cargo_version: $cargo_version
        rustc_version: $rustc_version
        script_sha256: $script_sha256
        cargo_lock_sha256: (open --raw Cargo.lock | hash sha256)
        flake_lock_sha256: (open --raw flake.lock | hash sha256)
        macos: (
            require-success "sw_vers" (run-command "sw_vers" [] {} $hidden_environment)
            | str trim
        )
        hardware: (
            require-success "sysctl hw.model" ((run-command
                "sysctl"
                ["-n" "hw.model"]
                {}
                $hidden_environment
            ))
            | str trim
        )
        cargo_incremental: "CARGO_INCREMENTAL unset; profile settings and Cargo configuration remain in effect"
        workspace_wrapper: "RUSTC_WORKSPACE_WRAPPER is empty for child Cargo processes"
        cache_limit_bytes: 2147483648
        temporary_root: ($temp_root | into string)
        result: $run
    }
    $data | to json --indent 2 | save --force $output_path

    if $run.status != "ok" {
        error make {
            msg: $"Measurement failed; diagnostic was written to ($output_path), but completed case measurements are unavailable:\n($run.error)"
        }
    }

    print --stderr $"Wrote measurement to ($output_path)"
    print --raw (open --raw $output_path)
}
