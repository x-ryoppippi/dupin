use std::{hint::black_box, time::Instant};

use serde_json::json;
use wasmi::{CompilationMode, Config, Engine, Linker, Module, Store, StoreLimitsBuilder, TrapCode};

const QUERY_FUEL: u64 = 20_000_000;
const ITERATIONS: u32 = 1_000;
const OUTPUT_CAP: i32 = 128;

fn engine() -> Engine {
    let mut config = Config::default();
    config.consume_fuel(true);
    config.compilation_mode(CompilationMode::LazyTranslation);
    Engine::new(&config)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("pass the calculator Wasm path")?;
    let wasm = std::fs::read(path)?;
    let engine = engine();
    let started = Instant::now();
    let module = Module::new(&engine, &wasm)?;
    let load_ns = started.elapsed().as_nanos();
    assert_eq!(
        module.imports().count(),
        0,
        "the bare guest must have no ambient imports"
    );
    let limits = StoreLimitsBuilder::new()
        .memory_size(16 * 1024 * 1024)
        .instances(1)
        .tables(1)
        .memories(1)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(&engine, limits);
    store.limiter(|limits| limits);
    store.set_fuel(QUERY_FUEL)?;
    let started = Instant::now();
    let instance = Linker::new(&engine).instantiate_and_start(&mut store, &module)?;
    let instantiate_ns = started.elapsed().as_nanos();
    let version = instance.get_typed_func::<(), i32>(&store, "abi_version")?;
    assert_eq!(version.call(&mut store, ())?, 1);
    let alloc = instance.get_typed_func::<i32, i32>(&store, "dupin_alloc")?;
    let free = instance.get_typed_func::<(i32, i32), ()>(&store, "dupin_free")?;
    let eval = instance.get_typed_func::<(i32, i32, i32, i32), i32>(&store, "dupin_eval")?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or("missing guest memory")?;
    let mut measurements = Vec::new();
    for query in ["560 km to mi", "30*(60+1)", "60 mph in m/s", "1 acre in m^2"] {
        let expected = dupin_calculator_guest::evaluate(query);
        assert!(!expected.is_empty(), "native evaluation failed for {query}");
        let input_ptr = alloc.call(&mut store, query.len() as i32)?;
        let output_ptr = alloc.call(&mut store, OUTPUT_CAP)?;
        assert!(input_ptr > 0 && output_ptr > 0);
        memory.write(&mut store, input_ptr as usize, query.as_bytes())?;
        store.set_fuel(QUERY_FUEL)?;
        let started = Instant::now();
        let len = eval.call(
            &mut store,
            (input_ptr, query.len() as i32, output_ptr, OUTPUT_CAP),
        )?;
        let first_ns = started.elapsed().as_nanos();
        assert!((1..=OUTPUT_CAP).contains(&len), "guest returned status {len}");
        let mut result = vec![0; len as usize];
        memory.read(&store, output_ptr as usize, &mut result)?;
        assert_eq!(String::from_utf8(result)?, expected);
        let started = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(dupin_calculator_guest::evaluate(black_box(query)));
        }
        let native_ns = started.elapsed().as_nanos() / u128::from(ITERATIONS);
        let started = Instant::now();
        let mut max_fuel = 0;
        for _ in 0..ITERATIONS {
            memory.write(&mut store, input_ptr as usize, query.as_bytes())?;
            store.set_fuel(QUERY_FUEL)?;
            let len = eval.call(
                &mut store,
                (input_ptr, query.len() as i32, output_ptr, OUTPUT_CAP),
            )?;
            assert!((1..=OUTPUT_CAP).contains(&len));
            let mut result = vec![0; len as usize];
            memory.read(&store, output_ptr as usize, &mut result)?;
            black_box(result);
            max_fuel = max_fuel.max(QUERY_FUEL - store.get_fuel()?);
        }
        let wasm_ns = started.elapsed().as_nanos() / u128::from(ITERATIONS);
        measurements.push(json!({
            "query": query, "result": expected, "first_wasm_ns": first_ns,
            "warm_native_mean_ns": native_ns, "warm_wasm_mean_ns": wasm_ns,
            "max_warm_fuel": max_fuel
        }));
        free.call(&mut store, (input_ptr, query.len() as i32))?;
        free.call(&mut store, (output_ptr, OUTPUT_CAP))?;
    }
    let query = "2^1000000";
    let input = alloc.call(&mut store, query.len() as i32)?;
    let output = alloc.call(&mut store, OUTPUT_CAP)?;
    memory.write(&mut store, input as usize, query.as_bytes())?;
    store.set_fuel(QUERY_FUEL)?;
    let error = eval
        .call(&mut store, (input, query.len() as i32, output, OUTPUT_CAP))
        .expect_err("pathological expression must exhaust fuel");
    assert_eq!(error.as_trap_code(), Some(TrapCode::OutOfFuel));
    store.set_fuel(QUERY_FUEL)?;
    let current_pages = memory.size(&store);
    memory.grow(&mut store, 256 - current_pages)?;
    assert_eq!(memory.size(&store), 256);
    assert!(
        memory.grow(&mut store, 1).is_err(),
        "growth beyond 16 MiB must fail"
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "wasm_bytes": wasm.len(), "module_load_ns": load_ns,
            "instantiate_ns": instantiate_ns, "iterations": ITERATIONS,
            "compilation_mode": "LazyTranslation",
            "query_fuel_limit": QUERY_FUEL, "pathological_fuel_limit": QUERY_FUEL,
            "memory_limit_bytes": 16 * 1024 * 1024, "queries": measurements,
            "fuel_cutoff_verified": true, "memory_growth_limit_verified": true
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trap(body: &str, fuel: u64) -> TrapCode {
        let engine = engine();
        let wasm = wat::parse_str(format!("(module (func $run (export \"run\") {body}))")).unwrap();
        let module = Module::new(&engine, &wasm).unwrap();
        let mut store = Store::new(&engine, ());
        store.set_fuel(fuel).unwrap();
        let instance = Linker::new(&engine)
            .instantiate_and_start(&mut store, &module)
            .unwrap();
        instance
            .get_typed_func::<(), ()>(&store, "run")
            .unwrap()
            .call(&mut store, ())
            .unwrap_err()
            .as_trap_code()
            .unwrap()
    }

    #[test]
    fn an_infinite_loop_exhausts_fuel() {
        assert_eq!(trap("(loop (br 0))", QUERY_FUEL), TrapCode::OutOfFuel);
    }

    #[test]
    fn guest_recursion_traps_without_overflowing_the_native_stack() {
        assert_eq!(trap("(call $run)", QUERY_FUEL), TrapCode::StackOverflow);
    }
}
