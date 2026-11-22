use super::*;
use dupin_plugin_protocol::{Outcome, PluginItem, VERSION, View};

const ALLOC: &str = "global.get $next global.get $next local.get 0 i32.add global.set $next";

fn request() -> Request {
    Request {
        version: VERSION,
        request_id: "request-1".into(),
        generation: "18446744073709551615".into(),
        command: "calculator".into(),
        query: "1+2".into(),
        limit: 1,
        state: None,
    }
}

fn response() -> Response {
    Response {
        version: VERSION,
        request_id: request().request_id,
        generation: request().generation,
        outcome: Outcome::Results {
            items: vec![PluginItem {
                id: "answer".into(),
                view: View::Pair {
                    source: "1+2".into(),
                    result: "3".into(),
                },
                actions: vec![],
            }],
            state: None,
        },
    }
}

fn fixture(version: &str, alloc: &str, query: &str, data: &[u8]) -> GuestModule {
    let data: String = data.iter().map(|byte| format!("\\{byte:02x}")).collect();
    let module = format!(
        r#"(module
        (memory (export "memory") 8)
        (global $next (mut i32) (i32.const 1024))
        (global $calls (mut i32) (i32.const 0))
        (func (export "dupin_abi_version") (result i32) {version})
        (func (export "dupin_alloc") (param i32) (result i32) {alloc})
        (func (export "dupin_query") (param i32 i32 i32 i32) (result i32) {query})
        (data (i32.const 400000) "{data}"))"#
    );
    GuestModule::load(&wat::parse_str(module).unwrap()).unwrap()
}

fn replying(bytes: &[u8]) -> GuestModule {
    fixture(
        "i32.const 1",
        ALLOC,
        &format!(
            "local.get 2 i32.const 400000 i32.const {} memory.copy i32.const {}",
            bytes.len(),
            bytes.len()
        ),
        bytes,
    )
}

#[test]
fn response_is_owned_and_each_query_has_a_fresh_instance() {
    let bytes = response().encode(&request()).unwrap();
    let module = fixture(
        "global.get $calls i32.const 1 i32.add global.set $calls global.get $calls",
        ALLOC,
        &format!(
            "local.get 2 i32.const 400000 i32.const {} memory.copy i32.const {}",
            bytes.len(),
            bytes.len()
        ),
        &bytes,
    );
    let first = module.query(&request()).unwrap();
    let second = module.query(&request()).unwrap();
    drop(module);
    assert_eq!(first, response());
    assert_eq!(second, first);
}

#[test]
fn arbitrary_pointer_values_and_overlapping_buffers_are_rejected() {
    for allocator in [
        "i32.const 0",
        "i32.const -1",
        "i32.const 2147483647",
        "i32.const 1024",
    ] {
        let module = fixture("i32.const 1", allocator, "i32.const 0", b"");
        assert_eq!(module.query(&request()), Err(HostError::Allocation));
    }
}

#[test]
fn output_lengths_are_checked_before_reading_or_allocating() {
    for len in [-1, 0, MAX_RESPONSE_BYTES as i32 + 1, i32::MAX] {
        let module = fixture("i32.const 1", ALLOC, &format!("i32.const {len}"), b"");
        assert_eq!(module.query(&request()), Err(HostError::InvalidOutput));
    }
}

#[test]
fn malformed_and_foreign_responses_do_not_escape_the_boundary() {
    for bytes in [b"not-json-private-payload".as_slice(), b"\xff"] {
        let error = replying(bytes).query(&request()).unwrap_err();
        assert_eq!(error, HostError::Protocol(ProtocolError::InvalidJson));
        assert!(!error.to_string().contains("private-payload"));
    }
    let mut other = request();
    other.generation = "another-generation".into();
    let bytes = response().encode(&request()).unwrap();
    assert_eq!(
        replying(&bytes).query(&other),
        Err(HostError::Protocol(ProtocolError::CorrelationMismatch))
    );
}

#[test]
fn fuel_stops_query_and_startup_loops() {
    let module = fixture("i32.const 1", ALLOC, "(loop $spin br $spin) i32.const 0", b"");
    assert_eq!(module.query(&request()), Err(HostError::Fuel));
    let bytes = wat::parse_str("(module (func $start (loop $spin br $spin)) (start $start))").unwrap();
    let module = GuestModule::load(&bytes).unwrap();
    assert_eq!(module.query(&request()), Err(HostError::Fuel));
}

#[test]
fn guest_recursion_traps_without_overflowing_the_native_stack() {
    let bytes =
        wat::parse_str("(module (func $recurse call $recurse) (func $start call $recurse) (start $start))")
            .unwrap();
    let module = GuestModule::load(&bytes).unwrap();
    assert_eq!(module.query(&request()), Err(HostError::Instantiate));
}

#[test]
fn memory_and_table_limits_cover_startup_and_growth() {
    for source in ["(module (memory 257))", "(module (table 1025 funcref))"] {
        let module = GuestModule::load(&wat::parse_str(source).unwrap()).unwrap();
        assert_eq!(module.query(&request()), Err(HostError::Instantiate));
    }
    let bytes = response().encode(&request()).unwrap();
    let copy = format!(
        "local.get 2 i32.const 400000 i32.const {} memory.copy i32.const {}",
        bytes.len(),
        bytes.len()
    );
    let exact = fixture(
        "i32.const 1",
        ALLOC,
        &format!("i32.const 248 memory.grow drop {copy}"),
        &bytes,
    );
    assert_eq!(exact.query(&request()).unwrap(), response());
    let module = fixture(
        "i32.const 1",
        ALLOC,
        "i32.const 249 memory.grow drop i32.const 0",
        b"",
    );
    assert_eq!(module.query(&request()), Err(HostError::Trap));
}

#[test]
fn imports_missing_exports_and_unknown_abi_are_rejected() {
    let bytes = wat::parse_str("(module (import \"wasi_snapshot_preview1\" \"fd_write\" (func)))").unwrap();
    assert!(matches!(GuestModule::load(&bytes), Err(HostError::Imports)));
    let module = GuestModule::load(&wat::parse_str("(module)").unwrap()).unwrap();
    assert_eq!(module.query(&request()), Err(HostError::Exports));
    let module = fixture("i32.const 2", ALLOC, "i32.const 0", b"");
    assert_eq!(module.query(&request()), Err(HostError::AbiVersion));
    assert!(matches!(
        GuestModule::load(&vec![0; MAX_MODULE_BYTES + 1]),
        Err(HostError::ModuleTooLarge)
    ));
}
