const edits = [
    {
        name: "dupin UI, function body"
        file: "crates/dupin/src/ui/keys.rs"
        literal: "query"
    }
    {
        name: "dupin-core session, function body"
        file: "crates/dupin-core/src/session/search.rs"
        literal: "Search unavailable"
    }
    {
        name: "dupin-mac, function body"
        file: "crates/dupin-mac/src/apps.rs"
        literal: "Applications"
    }
    {name: "docs only", file: "docs/development.md"}
    {name: "release version", file: "VERSION", replace: "9.9.9\n"}
]

const checks = ["lint-and-test" "hawk" "dupin"]

def edit-file [edit: record, original: string] {
    let marker = random uuid
    if $edit.replace? != null {
        $edit.replace
    } else if $edit.literal? != null {
        let quoted = $"\"($edit.literal)\""
        if not ($original | str contains $quoted) {
            error make {msg: $"($edit.file) no longer contains ($quoted)"}
        }
        $original | str replace $quoted $"\"($edit.literal) ($marker)\""
    } else {
        $original + $"\n<!-- ($marker) -->\n"
    }
}

def installable [check: string] {
    $".#checks.aarch64-darwin.($check)"
}

# Derivation names Nix would build for an installable, without building them.
def planned [check: string] {
    let result = ^nix build --dry-run --no-link (installable $check) | complete
    $result.stderr
    | lines
    | skip until {|line| $line =~ 'will be built' }
    | skip 1
    | take while {|line| $line starts-with "  " }
    | each { str trim | path basename | str replace --regex '^[a-z0-9]{32}-' '' }
}

def build [check: string] {
    let measured = timeit --output { ^nix build --no-link (installable $check) | complete }
    if $measured.output.exit_code != 0 {
        error make {
            msg: $"nix build (installable $check) failed:\n($measured.output.stderr)"
        }
    }
    ($measured.time | into int) / 1_000_000_000
}

# Times each Rust check after representative edits, one check at a time, from a store that
# already holds the unedited outputs.
def main [--output: path] {
    const script = path self
    for check in $checks {
        build $check | ignore
    }

    let measurements = $edits | each {|edit|
        let original = open --raw $edit.file
        edit-file $edit $original | save --force --raw $edit.file
        let result = try {
            $checks | each {|check|
                let derivations = planned $check
                let seconds = build $check
                print --stderr $"($check) after ($edit.name): ($seconds | math round --precision 1) s"
                {check: $check seconds: $seconds built: $derivations}
            }
        } catch {|error|
            $original | save --force --raw $edit.file
            error make {msg: $error.msg}
        }
        $original | save --force --raw $edit.file
        {edit: $edit.name file: $edit.file checks: $result}
    }

    let record = {
        commit: (^git rev-parse HEAD | str trim)
        script_sha256: (open --raw $script | hash sha256)
        cargo_lock_sha256: (open --raw Cargo.lock | hash sha256)
        nix: (^nix --version | str trim)
        host: (sys host | select --optional name os_version kernel_version)
        cpus: (sys cpu | length)
        measurements: $measurements
    }
    if $output == null {
        $record | to json
    } else {
        $record | to json | save --force $output
    }
}
