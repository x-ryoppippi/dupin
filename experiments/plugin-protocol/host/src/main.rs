use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

use dupin_plugin_protocol::{MAX_REQUEST_BYTES, Request};
use dupin_plugin_runtime::{GuestModule, MAX_MODULE_BYTES};

fn read(path: &Path, cap: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(cap as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > cap {
        return Err(std::io::Error::other("file exceeds byte limit"));
    }
    Ok(bytes)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("dupin-protocol-host <guest.wasm> <request.json>");
        return Ok(());
    }
    if args.len() == 1 && args[0] == "--version" {
        println!("dupin-protocol-host {} (experimental)", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.len() != 2 {
        return Err("pass guest.wasm and request.json".into());
    }
    if args.iter().any(|arg| arg.to_string_lossy().starts_with('-')) {
        return Err("unknown option; use --help".into());
    }
    let request = Request::decode(&read(Path::new(&args[1]), MAX_REQUEST_BYTES)?)?;
    let module = GuestModule::load(&read(Path::new(&args[0]), MAX_MODULE_BYTES)?)?;
    let response = module.query(&request)?;
    let bytes = response.encode(&request)?;
    std::io::stdout().write_all(&bytes)?;
    println!();
    Ok(())
}
