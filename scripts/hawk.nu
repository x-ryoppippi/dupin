def main [] {
    ^cargo-hawk check --manifest-path Cargo.toml --exclude-crate dupin_plugin_protocol --exclude-crate dupin_plugin_runtime --exclude-crate dupin_plugin_policy --exclude-crate dupin_extension_http -D warnings
}
