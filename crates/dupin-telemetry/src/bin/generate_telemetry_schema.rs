//! Writes the collector's ping schema module to each path given.

use std::{env, fs, io};

fn main() -> io::Result<()> {
    let module = dupin_telemetry::generate_schema_module();
    let paths: Vec<String> = env::args().skip(1).collect();
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected at least one output path",
        ));
    }
    for path in paths {
        fs::write(&path, &module)
            .map_err(|error| io::Error::new(error.kind(), format!("couldn't write {path}: {error}")))?;
    }
    Ok(())
}
