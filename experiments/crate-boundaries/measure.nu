# A `literal` edit changes a string inside a function body, so that code is generated again; the
# others append an unused item, most of which rustc's incremental cache skips.
const edits_by_name = [
    {name: "dupin UI, new item", file: "crates/dupin/src/ui/keys.rs"}
    {
        name: "dupin UI, function body"
        file: "crates/dupin/src/ui/keys.rs"
        literal: "query"
    }
    {
        name: "dupin-core session, new item"
        file: "crates/dupin-core/src/session/search.rs"
    }
    {
        name: "dupin-core session, function body"
        file: "crates/dupin-core/src/session/search.rs"
        literal: "Search unavailable"
    }
    {name: "dupin-mac, new item", file: "crates/dupin-mac/src/apps.rs"}
    {
        name: "dupin-mac, function body"
        file: "crates/dupin-mac/src/apps.rs"
        literal: "Applications"
    }
    {name: "docs only", file: "docs/development.md"}
]

const commands_by_name = [
    {
        name: "build"
        args: ["build" "--locked" "--package" "dupin"]
    }
    {
        name: "test"
        args: ["test" "--workspace" "--locked" "--no-run"]
    }
    {
        name: "clippy"
        args: ["clippy" "--workspace" "--all-targets" "--locked"]
    }
    {
        name: "release"
        args: ["build" "--locked" "--release" "--package" "dupin"]
    }
]

def workspace-packages [] {
    ^cargo metadata --format-version 1 --no-deps --locked | from json | get packages.name
}

def cargo-run [args: list<string>, target_dir: path] {
    let command = $args ++ [
        "--target-dir" ($target_dir | into string)
        "--message-format=json"
    ]
    let measured = timeit --output { ^cargo ...$command | complete }
    let result = $measured.output
    if $result.exit_code != 0 {
        error make {msg: $"cargo ($command | str join ' ') failed:\n($result.stderr)"}
    }
    let rebuilt = $result.stdout
    | lines
    | where {|line| $line starts-with "{" }
    | each { from json }
    | where reason == "compiler-artifact" and fresh == false
    | each {|artifact| $"($artifact.target.name) \(($artifact.target.kind | str join ','))" }
    {
        seconds: (($measured.time | into int) / 1_000_000_000)
        rebuilt: $rebuilt
    }
}

def probe [edit: record, original: string] {
    let marker = random uuid
    if $edit.literal? != null {
        let quoted = $"\"($edit.literal)\""
        if not ($original | str contains $quoted) {
            error make {msg: $"($edit.file) no longer contains ($quoted)"}
        }
        $original | str replace $quoted $"\"($edit.literal) ($marker)\""
    } else if ($edit.file | str ends-with ".rs") {
        $original + $"\nconst _EDIT_PROBE: &str = \"($marker)\";\n"
    } else {
        $original + $"\n<!-- ($marker) -->\n"
    }
}

# Times representative source edits against an already-built target directory.
def main [
    --target-dir: path = "target"
    --repeat: int = 3
    --commands: string = "^(build|test|clippy)$" # a regular expression that selects commands by name
    --edits: string = "" # a regular expression that selects edits by name
    --output: path
] {
    const script = path self
    let commands = $commands_by_name | where name =~ $commands
    let edits = $edits_by_name | where name =~ $edits
    let target_dir = $target_dir | path expand
    let workspace = workspace-packages | each { str replace --all "-" "_" }

    print --stderr "Warming every command, then checking that alternating them reuses each other's units"
    for command in $commands {
        cargo-run $command.args $target_dir | ignore
    }
    let alternation = $commands | each {|command|
        let run = cargo-run $command.args $target_dir
        {command: $command.name seconds: $run.seconds rebuilt: $run.rebuilt}
    }

    let measurements = $commands | each {|command|
        # Units only this command builds are still stale from the previous command's edits.
        cargo-run $command.args $target_dir | ignore
        $edits | each {|edit|
            let original = open --raw $edit.file
            let runs = 1..$repeat | each {
                probe $edit $original | save --force --raw $edit.file
                let run = try { cargo-run $command.args $target_dir } catch {|error|
                    $original | save --force --raw $edit.file
                    error make {msg: $error.msg}
                }
                $original | save --force --raw $edit.file
                cargo-run $command.args $target_dir | ignore
                print --stderr $"($command.name) after ($edit.name): ($run.seconds | math round --precision 2) s"
                $run
            }
            {
                command: $command.name
                edit: $edit.name
                file: $edit.file
                seconds: ($runs | get seconds)
                median_seconds: ($runs | get seconds | math median)
                rebuilt: ($runs | first | get rebuilt)
                rebuilt_dependencies: (
                    $runs | first | get rebuilt
                    | where {|unit| ($unit | split row " " | first) not-in $workspace }
                )
            }
        }
    } | flatten

    let record = {
        commit: (^git rev-parse HEAD | str trim)
        dirty: ((^git status --porcelain --untracked-files=no | str trim) | is-not-empty)
        script_sha256: (open --raw $script | hash sha256)
        cargo_lock_sha256: (open --raw Cargo.lock | hash sha256)
        rustc: (^rustc --version | str trim)
        host: (sys host | select --optional name os_version kernel_version)
        cpus: (sys cpu | length)
        repeat: $repeat
        cargo_incremental: ($env.CARGO_INCREMENTAL? | default "profile default")
        alternation: $alternation
        measurements: $measurements
    }
    if $output == null {
        $record | to json
    } else {
        $record | to json | save --force $output
    }
}
