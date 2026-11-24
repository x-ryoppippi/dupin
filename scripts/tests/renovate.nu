#!/usr/bin/env nu

let config = open .github/renovate.json

# Whether the rules that `matches` selects leave automerge on, the last one that sets it winning.
def automerges [config: record, start: bool, matches: closure]: nothing -> bool {
    $config.packageRules
    | where $matches
    | where {|rule| 'automerge' in ($rule | columns) }
    | reduce --fold $start {|rule, _| $rule.automerge }
}

let default = $config.automerge? | default false

let lock_file = (automerges
    $config
    ($config.lockFileMaintenance.automerge? | default $default)
    {|rule| 'lockFileMaintenance' in ($rule.matchUpdateTypes? | default []) }
)
if $lock_file {
    error make {
        msg: 'Renovate must not automerge lock file maintenance: it skips the release-age gate, and main ships to the signed nightly feed.'
    }
}

let rune = (automerges
    $config
    $default
    {|rule| 'rune' in ($rule.matchDepNames? | default []) }
)
if $rune {
    error make {
        msg: 'Renovate must not automerge Rune: a git-refs digest skips the release-age gate, and main ships to the signed nightly feed.'
    }
}
