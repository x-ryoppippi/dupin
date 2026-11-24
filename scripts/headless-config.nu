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
    {XDG_CONFIG_HOME: $config_dir}
}
