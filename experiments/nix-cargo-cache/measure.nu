def checked [program: string, args: list<string>] {
    let result = run-external $program ...$args | complete
    if $result.exit_code != 0 {
        error make {msg: $"($program) failed:\n($result.stderr)\n($result.stdout)"}
    }
    $result.stdout
}

def evaluate [flake: string, attribute: string] {
    checked "nix" ["eval" "--no-write-lock-file" "--raw" $"($flake)#($attribute)"] | str trim
}

def build [
    root: path
    label: string
    flake: string
    attribute: string
    offline: bool
] {
    print --stderr $"Building ($label)"
    let link = $root | path join roots $label
    let args = [
        "build"
        "--no-write-lock-file"
        "--json"
        "--out-link" $link
        $"($flake)#($attribute)"
    ]
    | append (if $offline { ["--offline"] } else { [] })
    let timed = timeit --output { ^nix ...$args | complete }
    $timed.output.stderr | save --force ($root | path join logs $"($label).log")
    if $timed.output.exit_code != 0 {
        error make {
            msg: $"($label) failed; see ($root)/logs/($label).log:\n($timed.output.stderr)"
        }
    }
    {
        label: $label
        command: (["nix"] ++ $args)
        elapsed_ns: ($timed.time | into int)
        outputs: ($timed.output.stdout | from json)
    }
}

def compiler-inputs [flake: string, attribute: string] {
    let expression = 'inputs: map (p: { name = p.name; path = toString p; }) (builtins.filter (p: builtins.match "(auditable-cargo|rustc-wrapper).*" p.name != null) inputs)'
    checked "nix" [
        "eval"
        "--no-write-lock-file"
        "--json" $"($flake)#($attribute).nativeBuildInputs"
        "--apply" $expression
    ]
    | from json
    | sort-by name
}

def application [flake: string, audit_tool: path] {
    let output = evaluate $flake "dupin.outPath"
    let binary = $output | path join Applications Dupin.app Contents MacOS dupin
    let audit = checked ($audit_tool | into string) [$binary] | from json
    {
        output: $output
        binary_bytes: (ls $binary | get 0.size | into int)
        binary_sha256: (open --raw $binary | hash sha256)
        references: (checked "nix-store" ["--query" "--references" $output] | lines | sort)
        audit: $audit
    }
}

def main [
    audit_tool: path
    --baseline-revision: string
    --revision: string = "HEAD"
    --output: path = "/tmp/dupin-nix-cache-results.json"
] {
    const script = path self
    if ($baseline_revision | is-empty) {
        error make {msg: "Provide --baseline-revision with the original Nix build."}
    }
    let audit_tool = $audit_tool | path expand
    let output = $output | path expand
    if not ($audit_tool | path exists) or not ($output | path dirname | path exists) {
        error make {msg: "The audit tool and output parent directory must exist."}
    }
    if ($output | path type) == "dir" {
        error make {msg: "The output path must be a file."}
    }
    let started_at = date now | date to-timezone UTC
    let repository = checked "git" ["rev-parse" "--show-toplevel"] | str trim
    cd $repository
    let baseline_revision = checked "git" ["rev-parse" "--verify" $"($baseline_revision)^{commit}"] | str trim
    let revision = checked "git" ["rev-parse" "--verify" $"($revision)^{commit}"] | str trim
    let sources = [
        "Cargo.toml"
        "Cargo.lock"
        "crates"
        "examples/shaders"
        "packaging"
        "assets/logo"
        "VERSION"
        "rustfmt.toml"
    ]
    let changed = checked "git" (["diff" "--name-only" $baseline_revision $revision "--"] ++ $sources) | str trim
    if ($changed | is-not-empty) {
        error make {
            msg: $"The build/packaging sources must match between revisions:\n($changed)"
        }
    }

    let root = mktemp --directory --tmpdir-path "/tmp" "dupin-nix-cache.XXXXXXXX" | path expand
    mkdir ($root | path join logs) ($root | path join roots)
    let nix_version = checked "nix" ["--version"] | str trim
    let macos = checked "sw_vers" [] | str trim
    let hardware = checked "sysctl" ["-n" "hw.model"] | str trim
    let snapshots = [
        {name: baseline, revision: $baseline_revision}
        {name: candidate, revision: $revision}
    ] | each {|snapshot|
        let archive = $root | path join $"($snapshot.name).tar"
        let directory = $root | path join $snapshot.name
        mkdir $directory
        checked "git" ["archive" "--format=tar" "--output" $archive $snapshot.revision] | ignore
        checked "tar" ["-xf" $archive "-C" $directory] | ignore
        {name: $snapshot.name, directory: $directory, flake: $"path:($directory)"}
    }
    let baseline = $snapshots | where name == baseline | get 0
    let candidate = $snapshots | where name == candidate | get 0
    let baseline_lock = open --raw ($baseline.directory | path join flake.lock) | from json
    let candidate_lock = open --raw ($candidate.directory | path join flake.lock) | from json
    let nixpkgs = $baseline_lock.nodes | get $baseline_lock.nodes.root.inputs.nixpkgs | get locked
    let candidate_nixpkgs = $candidate_lock.nodes | get $candidate_lock.nodes.root.inputs.nixpkgs | get locked
    if $nixpkgs != $candidate_nixpkgs {
        error make {msg: "The two revisions must pin the same nixpkgs."}
    }
    let baseline_tools = compiler-inputs $baseline.flake "dupin"
    let candidate_tools = compiler-inputs $candidate.flake "dupin.cargoBinary"
    if ($baseline_tools | length) != 2 or $baseline_tools != $candidate_tools {
        error make {msg: "The audited Cargo and rustc store paths must match."}
    }

    let run = try {
        let before_artifacts = {
            release: (evaluate $candidate.flake "dupin.releaseArtifacts.drvPath")
            debug: (evaluate $candidate.flake "dupin.debugArtifacts.drvPath")
        }
        let resource_before = {
            binary: (evaluate $candidate.flake "dupin.cargoBinary.drvPath")
            package: (evaluate $candidate.flake "dupin.drvPath")
            artifacts: $before_artifacts
        }
        let template = $candidate.directory | path join packaging Info.plist
        let original_template = open --raw $template
        $"\n<!-- cache probe (random uuid) -->\n" | save --append $template
        let resource_after = {
            binary: (evaluate $candidate.flake "dupin.cargoBinary.drvPath")
            package: (evaluate $candidate.flake "dupin.drvPath")
            artifacts: {
                release: (evaluate $candidate.flake "dupin.releaseArtifacts.drvPath")
                debug: (evaluate $candidate.flake "dupin.debugArtifacts.drvPath")
            }
        }
        $original_template | save --force $template
        let restored_package = evaluate $candidate.flake "dupin.drvPath"
        if $resource_before.binary != $resource_after.binary or $resource_before.artifacts != $resource_after.artifacts or $resource_before.package == $resource_after.package or $restored_package != $resource_before.package {
            error make {
                msg: "A packaging-only edit must change the bundle without invalidating Rust builds."
            }
        }
        let attributes = [
            {label: release, attribute: "dupin"}
            {label: checks, attribute: "checks.aarch64-darwin.lint-and-test"}
        ]
        let before_derivations = $snapshots | each {|snapshot|
            {
                name: $snapshot.name
                values: ($attributes | each {|attribute|
                    {label: $attribute.label, drv: (evaluate $snapshot.flake $"($attribute.attribute).drvPath")}
                })
            }
        }
        let preparation = [
            (
                (build
                    $root
                    "release-cache"
                    $candidate.flake
                    "dupin.releaseArtifacts"
                    false
                )
            )
            (
                (build
                    $root
                    "debug-cache"
                    $candidate.flake
                    "dupin.debugArtifacts"
                    false
                )
            )
        ] ++ ($snapshots | each {|snapshot|
            $attributes | each {|attribute|
                build $root $"warm-($snapshot.name)-($attribute.label)" $snapshot.flake $attribute.attribute false
            }
        } | flatten)

        let nonce = random uuid
        let edit = $"\nconst _: &str = \"($nonce)\";\n"
        $snapshots | each {|snapshot|
            $edit | save --append ($snapshot.directory | path join crates dupin src ui.rs)
        } | ignore
        let after_artifacts = {
            release: (evaluate $candidate.flake "dupin.releaseArtifacts.drvPath")
            debug: (evaluate $candidate.flake "dupin.debugArtifacts.drvPath")
        }
        if $before_artifacts != $after_artifacts {
            error make {
                msg: "A Rust source edit changed the dependency artifact derivations."
            }
        }
        let after_derivations = $snapshots | each {|snapshot|
            {
                name: $snapshot.name
                values: ($attributes | each {|attribute|
                    {label: $attribute.label, drv: (evaluate $snapshot.flake $"($attribute.attribute).drvPath")}
                })
            }
        }
        let stale = $before_derivations | zip $after_derivations | any {|pair|
            $pair.0.values | zip $pair.1.values | any {|entry| $entry.0.drv == $entry.1.drv}
        }
        if $stale {
            error make {
                msg: "The Rust source edit must change every application/check derivation."
            }
        }
        let measurements = $attributes | each {|attribute|
            $snapshots | each {|snapshot|
                build $root $"edited-($snapshot.name)-($attribute.label)" $snapshot.flake $attribute.attribute true
            }
        } | flatten
        let baseline_app = application $baseline.flake $audit_tool
        let candidate_app = application $candidate.flake $audit_tool
        {
            status: ok
            resource_edit: {
                before: $resource_before
                after: $resource_after
                restored_package: $restored_package
            }
            edit: {file: "crates/dupin/src/ui.rs", contents: $edit}
            dependency_artifacts_before: $before_artifacts
            dependency_artifacts_after: $after_artifacts
            derivations_before: $before_derivations
            derivations_after: $after_derivations
            preparation: $preparation
            measurements: $measurements
            applications: {baseline: $baseline_app, candidate: $candidate_app}
            validation: {
                binary_size_equal: ($baseline_app.binary_bytes == $candidate_app.binary_bytes)
                audit_metadata_equal: ($baseline_app.audit == $candidate_app.audit)
                runtime_references_equal: ($baseline_app.references == $candidate_app.references)
            }
        }
    } catch {|error|
        {status: failed, error: $error.rendered}
    }
    let report = {
        format_version: 1
        started_at: $started_at
        baseline_revision: $baseline_revision
        revision: $revision
        script_sha256: (open --raw $script | hash sha256)
        nix_version: $nix_version
        nixpkgs: $nixpkgs
        compiler_inputs: $candidate_tools
        audit_tool: ($audit_tool | into string)
        macos: $macos
        hardware: $hardware
        temporary_root: ($root | into string)
        result: $run
    }
    $report | to json --indent 2 | save --force $output
    if $run.status != "ok" {
        error make {
            msg: $"Measurement failed; diagnostic written to ($output):\n($run.error)"
        }
    }
    if not ($run.validation | values | all {|value| $value}) {
        error make {msg: $"A static application comparison failed; inspect ($output)."}
    }
    print --stderr $"Wrote measurement to ($output); temporary sources and GC roots remain in ($root)"
}
