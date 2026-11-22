use std::sync::OnceLock;

use fend_core::{Context, Interrupt};

static CONTEXT: OnceLock<Context> = OnceLock::new();

struct NoGuestDeadline;

impl Interrupt for NoGuestDeadline {
    fn should_interrupt(&self) -> bool {
        false
    }
}

pub fn evaluate(query: &str) -> String {
    fend_core::evaluate_preview_with_interrupt(query, CONTEXT.get_or_init(Context::new), &NoGuestDeadline)
        .get_main_result()
        .to_owned()
}

#[unsafe(no_mangle)]
pub extern "C" fn abi_version() -> i32 {
    1
}

#[unsafe(no_mangle)]
#[cfg(target_arch = "wasm32")]
pub extern "C" fn dupin_alloc(len: i32) -> i32 {
    if len <= 0 || len > 1_048_576 {
        return 0;
    }
    let boxed = vec![0_u8; len as usize].into_boxed_slice();
    Box::into_raw(boxed) as *mut u8 as i32
}

#[unsafe(no_mangle)]
#[cfg(target_arch = "wasm32")]
/// # Safety
/// On wasm32, `ptr` and `len` must identify a live allocation from `dupin_alloc`.
/// It must not be used again after this call.
pub unsafe extern "C" fn dupin_free(ptr: i32, len: i32) {
    if ptr != 0 && len > 0 {
        let slice = std::ptr::slice_from_raw_parts_mut(ptr as *mut u8, len as usize);
        unsafe {
            drop(Box::from_raw(slice));
        }
    }
}

#[unsafe(no_mangle)]
#[cfg(target_arch = "wasm32")]
/// # Safety
/// On wasm32, both pointers must refer to live, disjoint `dupin_alloc` buffers
/// at least as large as their supplied lengths. They must remain live throughout
/// the call. This fixture ABI is not supported on native targets.
pub unsafe extern "C" fn dupin_eval(input_ptr: i32, input_len: i32, output_ptr: i32, output_cap: i32) -> i32 {
    if input_ptr == 0 || input_len <= 0 || input_len > 256 || output_ptr == 0 || output_cap <= 0 {
        return -1;
    }
    let input = unsafe { std::slice::from_raw_parts(input_ptr as *const u8, input_len as usize) };
    let Ok(query) = std::str::from_utf8(input) else {
        return -2;
    };
    let result = evaluate(query);
    if result.is_empty() || result.len() > output_cap as usize {
        return -3;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(result.as_ptr(), output_ptr as *mut u8, result.len());
    }
    result.len() as i32
}
