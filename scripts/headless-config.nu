export def scenario-env [script: path, tmp: path]: nothing -> record {
    let fixture = $script | path parse | update extension toml | path join
    if not ($fixture | path exists) {
        return {}
    }
    let name = $script | path parse | get stem
    let config_dir = $tmp | path join $"($name)-config"
    mkdir ($config_dir | path join dupin)
    cp $fixture ($config_dir | path join dupin config.toml)
    # A scenario's extensions live beside it, as `<name>.extensions/`.
    let extensions = $script | path parse | update extension extensions | path join
    if ($extensions | path exists) {
        cp --recursive $extensions ($config_dir | path join dupin extensions)
    }
    # And the grants it declares, as `<name>.grants.toml`.
    let grants = $script | path parse | update extension grants.toml | path join
    if ($grants | path exists) {
        cp $grants ($config_dir | path join dupin extension-grants.toml)
    }
    # And saved snapshots of its own, as `<name>.snapshots/archive.json`, in a data folder of its own.
    let snapshots = $script | path parse | update extension snapshots | path join
    let data = if ($snapshots | path exists) {
        let data_dir = $tmp | path join $"($name)-data"
        mkdir ($data_dir | path join dupin)
        cp --recursive $snapshots ($data_dir | path join dupin snapshots)
        {XDG_DATA_HOME: $data_dir}
    } else {
        {}
    }
    # And a home folder of its own, as `<name>.home/`: apps in `Applications`, and the usage
    # history in `Library/Application Support/Dupin/history.json`, where `$HOME` stands for the
    # copy's path, as the history names apps by path.
    let home = $script | path parse | update extension home | path join
    let data = if ($home | path exists) {
        let copy = $tmp | path join $"($name)-home"
        cp --recursive $home $copy
        let history = $copy | path join Library 'Application Support' Dupin history.json
        if ($history | path exists) {
            open --raw $history | str replace --all '$HOME' $copy | save --force $history
        }
        $data | merge {HOME: $copy}
    } else {
        $data
    }
    # And a clipboard history of its own, as `<name>.clipboard/history.json` beside the images it
    # names, in place of the shared one, which the latency baselines are fingerprinted by.
    let clipboard = $script | path parse | update extension clipboard | path join | path join history.json
    if ($clipboard | path exists) {
        {
            XDG_CONFIG_HOME: $config_dir
            DUPIN_HEADLESS_CLIPBOARD: ($clipboard | path expand)
        } | merge $data
    } else {
        {XDG_CONFIG_HOME: $config_dir} | merge $data
    }
}
