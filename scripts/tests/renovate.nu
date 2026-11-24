#!/usr/bin/env nu

let config = open .github/renovate.json
let default = $config.lockFileMaintenance.automerge? | default ($config.automerge? | default false)

let automerge = $config.packageRules
| where {|rule| 'lockFileMaintenance' in ($rule.matchUpdateTypes? | default []) }
| where {|rule| 'automerge' in ($rule | columns) }
| reduce --fold $default {|rule, _| $rule.automerge }

if $automerge {
    error make {
        msg: 'Renovate must not automerge lock file maintenance: it skips the release-age gate, and main ships to the signed nightly feed.'
    }
}
