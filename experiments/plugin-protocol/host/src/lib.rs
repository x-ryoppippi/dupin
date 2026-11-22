use std::{fmt, ops::Range};

use dupin_plugin_protocol::{MAX_RESPONSE_BYTES, ProtocolError, Request, Response};
use wasmi::{Config, Engine, Linker, Memory, Module, Store, StoreLimitsBuilder, TrapCode};

pub const MAX_MODULE_BYTES: usize = 4 * 1024 * 1024;
const MEMORY_BYTES: usize = 16 * 1024 * 1024;
const INIT_FUEL: u64 = 1_000_000;
const QUERY_FUEL: u64 = 20_000_000;

#[derive(Debug, PartialEq, Eq)]
pub enum HostError {
    ModuleTooLarge,
    InvalidModule,
    Imports,
    Instantiate,
    Exports,
    AbiVersion,
    Allocation,
    InvalidOutput,
    Memory,
    Fuel,
    Trap,
    Protocol(ProtocolError),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(f, "plugin protocol: {error}"),
            Self::ModuleTooLarge => f.write_str("plugin module exceeds byte limit"),
            Self::InvalidModule => f.write_str("invalid plugin module"),
            Self::Imports => f.write_str("plugin imports are unavailable"),
            Self::Instantiate => f.write_str("plugin instance could not be created within its limits"),
            Self::Exports => f.write_str("plugin ABI exports are missing or have incorrect types"),
            Self::AbiVersion => f.write_str("unsupported plugin ABI version"),
            Self::Allocation => f.write_str("plugin returned invalid or overlapping buffers"),
            Self::InvalidOutput => f.write_str("plugin returned an invalid output length or status"),
            Self::Memory => f.write_str("plugin memory access failed"),
            Self::Fuel => f.write_str("plugin exhausted its execution budget"),
            Self::Trap => f.write_str("plugin execution trapped"),
        }
    }
}

impl std::error::Error for HostError {}

fn execution(error: wasmi::Error) -> HostError {
    if error.as_trap_code() == Some(TrapCode::OutOfFuel) {
        HostError::Fuel
    } else {
        HostError::Trap
    }
}

fn region(pointer: i32, len: usize, memory_len: usize) -> Result<Range<usize>, HostError> {
    let start = usize::try_from(pointer).map_err(|_| HostError::Allocation)?;
    let end = start.checked_add(len).ok_or(HostError::Allocation)?;
    if start == 0 || end > memory_len {
        return Err(HostError::Allocation);
    }
    Ok(start..end)
}

pub struct GuestModule {
    engine: Engine,
    module: Module,
}

impl GuestModule {
    pub fn load(bytes: &[u8]) -> Result<Self, HostError> {
        if bytes.len() > MAX_MODULE_BYTES {
            return Err(HostError::ModuleTooLarge);
        }
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|_| HostError::InvalidModule)?;
        if module.imports().count() != 0 {
            return Err(HostError::Imports);
        }
        Ok(Self { engine, module })
    }

    pub fn query(&self, request: &Request) -> Result<Response, HostError> {
        let input = request.encode().map_err(HostError::Protocol)?;
        let limits = StoreLimitsBuilder::new()
            .memory_size(MEMORY_BYTES)
            .memories(1)
            .tables(1)
            .table_elements(1024)
            .instances(1)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|limits| limits);
        store.set_fuel(INIT_FUEL).map_err(execution)?;
        let instance = Linker::new(&self.engine)
            .instantiate_and_start(&mut store, &self.module)
            .map_err(|error| {
                if error.as_trap_code() == Some(TrapCode::OutOfFuel) {
                    HostError::Fuel
                } else {
                    HostError::Instantiate
                }
            })?;
        let version = instance
            .get_typed_func::<(), i32>(&store, "dupin_abi_version")
            .map_err(|_| HostError::Exports)?;
        if version.call(&mut store, ()).map_err(execution)? != 1 {
            return Err(HostError::AbiVersion);
        }
        let alloc = instance
            .get_typed_func::<i32, i32>(&store, "dupin_alloc")
            .map_err(|_| HostError::Exports)?;
        let query = instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&store, "dupin_query")
            .map_err(|_| HostError::Exports)?;
        let memory: Memory = instance.get_memory(&store, "memory").ok_or(HostError::Exports)?;
        let input_pointer = alloc.call(&mut store, input.len() as i32).map_err(execution)?;
        let input_range = region(input_pointer, input.len(), memory.data_size(&store))?;
        let output_pointer = alloc
            .call(&mut store, MAX_RESPONSE_BYTES as i32)
            .map_err(execution)?;
        let output_range = region(output_pointer, MAX_RESPONSE_BYTES, memory.data_size(&store))?;
        if input_range.start < output_range.end && output_range.start < input_range.end {
            return Err(HostError::Allocation);
        }
        memory
            .write(&mut store, input_range.start, &input)
            .map_err(|_| HostError::Memory)?;
        store.set_fuel(QUERY_FUEL).map_err(execution)?;
        let len = query
            .call(
                &mut store,
                (
                    input_pointer,
                    input.len() as i32,
                    output_pointer,
                    MAX_RESPONSE_BYTES as i32,
                ),
            )
            .map_err(execution)?;
        if len <= 0 || len as usize > MAX_RESPONSE_BYTES {
            return Err(HostError::InvalidOutput);
        }
        let mut bytes = vec![0; len as usize];
        memory
            .read(&store, output_range.start, &mut bytes)
            .map_err(|_| HostError::Memory)?;
        // The response owns its data; dropping this store releases every guest allocation,
        // including after a trap, without calling back into a damaged guest allocator.
        Response::decode(&bytes, request).map_err(HostError::Protocol)
    }
}

#[cfg(test)]
mod tests;
