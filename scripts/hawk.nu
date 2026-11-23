def main [] {
    ^cargo-hawk check --manifest-path Cargo.toml --exclude-crate dupin_plugin_protocol --exclude-crate dupin_plugin_runtime --exclude-crate dupin_broker --exclude-crate dupin_host_services -D warnings
}
