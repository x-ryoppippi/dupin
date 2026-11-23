const fixtures = path self fixtures/scripts

export def prepare-scripts [out: path] {
    let directory = $out | path join config dupin scripts
    let results = $out | path join script-fixtures
    if ($directory | path exists) or ($results | path exists) { error make {msg: 'Script fixtures already exist'} }
    mkdir $directory $results
    for name in [echo stream failure] {
        let target = $directory | path join $name
        cp ($fixtures | path join $name) $target
        ^chmod 700 $target
    }
}

export def script-processes-stopped [out: path]: nothing -> bool {
    let file = $out | path join script-fixtures processes.json
    if not ($file | path exists) { return true }
    let pids = open $file
    [$pids.parent $pids.child] | all {|pid|
        if ($pid | describe) != int or $pid <= 1 { error make {msg: 'Invalid fixture process id'} }
        let result = ^ps -p $pid -o stat= | complete
        if $result.exit_code == 1 { true } else if $result.exit_code != 0 {
            error make {msg: 'Could not inspect fixture process'}
        } else { $result.stdout | str trim | str starts-with Z }
    }
}

export def cleanup-scripts [out: path] {
    let directory = $out | path join config dupin scripts
    let results = $out | path join script-fixtures
    let marker = $results | path join processes.json
    if ($marker | path exists) {
        let pids = open $marker
        if ($pids.parent | describe) != int or $pids.parent <= 1 { error make {msg: 'Invalid fixture process group'} }
        let expected = $directory | path join stream
        let owned = [$pids.parent $pids.child] | any {|pid|
            if ($pid | describe) != int or $pid <= 1 { error make {msg: 'Invalid fixture process id'} }
            let command = ^ps -ww -p $pid -o command= | complete
            let group = ^ps -p $pid -o pgid= | complete
            $command.exit_code == 0 and $group.exit_code == 0 and ($command.stdout | str contains $expected) and ($group.stdout | str trim | into int) == $pids.parent
        }
        if $owned {
            ^/bin/kill -KILL $'-($pids.parent)' | complete | ignore
        } else if not (script-processes-stopped $out) {
            error make {
                msg: 'Refusing to signal a process outside the script fixture group'
            }
        }
    }
    if ($directory | path exists) { rm --recursive $directory }
}
