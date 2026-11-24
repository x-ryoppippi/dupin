//! Seals the bundled extensions: the content hash of each package folder in the repository's
//! `extensions/`, which the launcher compares a bundled folder with before trusting it.

use std::path::Path;

fn main() {
    let extensions = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions");
    println!("cargo::rerun-if-changed={}", extensions.display());
    let mut seals = Vec::new();
    for entry in std::fs::read_dir(&extensions).expect("read the repository's extensions/") {
        let folder = entry.expect("read the repository's extensions/").path();
        let name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a bundled package folder is named in UTF-8")
            .to_owned();
        if name.starts_with('.') || !folder.is_dir() {
            continue;
        }
        let hash = dupin_extension_install::content_hash(&folder)
            .unwrap_or_else(|error| panic!("hash {}: {error}", folder.display()));
        seals.push((name, hash));
    }
    seals.sort();
    let mut source = String::from("const SHIPPED: &[(&str, &str)] = &[\n");
    for (name, hash) in seals {
        source.push_str(&format!("    ({name:?}, {hash:?}),\n"));
    }
    source.push_str("];\n");
    let out = std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR");
    std::fs::write(Path::new(&out).join("bundled_seals.rs"), source).expect("write the seals");
}
