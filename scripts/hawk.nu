def main [] {
    ^cargo-hawk check --manifest-path Cargo.toml --exclude-crate dupin_plugin_protocol --exclude-crate dupin_broker --exclude-crate dupin_host_services --exclude-crate dupin_extension_http --exclude-crate dupin_telemetry -D warnings
}
