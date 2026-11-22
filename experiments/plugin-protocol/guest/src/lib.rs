use std::sync::OnceLock;

use dupin_plugin_protocol::{
    ErrorCode, Outcome, PluginItem, ProtocolError, Request, Response, VERSION, View,
};
#[cfg(target_arch = "wasm32")]
use dupin_plugin_protocol::{MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES};
use fend_core::{Context, Interrupt};

static CONTEXT: OnceLock<Context> = OnceLock::new();

struct HostFuel;

impl Interrupt for HostFuel {
    fn should_interrupt(&self) -> bool {
        // Instruction fuel is enforced by the host without giving the guest a clock import.
        false
    }
}

pub fn handle(bytes: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    handle_with(bytes, |query| {
        fend_core::evaluate_preview_with_interrupt(query, CONTEXT.get_or_init(Context::new), &HostFuel)
            .get_main_result()
            .to_owned()
    })
}

fn handle_with(bytes: &[u8], evaluate: impl FnOnce(&str) -> String) -> Result<Vec<u8>, ProtocolError> {
    let request = Request::decode(bytes)?;
    let outcome = if request.command != "calculator" {
        Outcome::Error {
            code: ErrorCode::UnsupportedCommand,
            message: "This guest only evaluates calculator queries.".into(),
        }
    } else if request.state.is_some() {
        Outcome::Error {
            code: ErrorCode::InvalidQuery,
            message: "Saved state is not supported by this guest.".into(),
        }
    } else if request.limit == 0 || request.query.trim().is_empty() {
        Outcome::Results {
            items: Vec::new(),
            state: None,
        }
    } else {
        let result = evaluate(&request.query);
        if result.is_empty() {
            Outcome::Results {
                items: Vec::new(),
                state: None,
            }
        } else {
            Outcome::Results {
                items: vec![PluginItem {
                    id: "answer".into(),
                    view: View::Pair {
                        source: request.query.clone(),
                        result,
                    },
                    actions: Vec::new(),
                }],
                state: None,
            }
        }
    };
    Response {
        version: VERSION,
        request_id: request.request_id.clone(),
        generation: request.generation.clone(),
        outcome,
    }
    .encode(&request)
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn dupin_abi_version() -> i32 {
    1
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn dupin_alloc(len: i32) -> i32 {
    if len <= 0 || len as usize > MAX_RESPONSE_BYTES {
        return 0;
    }
    Box::into_raw(vec![0_u8; len as usize].into_boxed_slice()) as *mut u8 as i32
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
/// # Safety
/// The host supplies disjoint live `dupin_alloc` buffers of the declared lengths.
/// Allocations live until this request's Wasm instance is discarded.
pub unsafe extern "C" fn dupin_query(input: i32, len: i32, output: i32, cap: i32) -> i32 {
    if input <= 0
        || output <= 0
        || len <= 0
        || len as usize > MAX_REQUEST_BYTES
        || cap <= 0
        || cap as usize > MAX_RESPONSE_BYTES
    {
        return -1;
    }
    // SAFETY: the host owns these allocations and keeps them alive for this call.
    let bytes = unsafe { std::slice::from_raw_parts(input as *const u8, len as usize) };
    let Ok(result) = handle(bytes) else {
        return -2;
    };
    if result.len() > cap as usize {
        return -3;
    }
    // SAFETY: `result` owns its allocation and the host's output buffer has enough room.
    unsafe {
        std::ptr::copy_nonoverlapping(result.as_ptr(), output as *mut u8, result.len());
    }
    result.len() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(query: &str) -> Request {
        Request {
            version: VERSION,
            request_id: "request-1".into(),
            generation: "18446744073709551615".into(),
            command: "calculator".into(),
            query: query.into(),
            limit: 1,
            state: None,
        }
    }

    #[test]
    fn calculator_returns_a_correlated_pair() {
        let request = request("30*(60+1)");
        let bytes = handle(&request.encode().unwrap()).unwrap();
        let response = Response::decode(&bytes, &request).unwrap();
        let Outcome::Results { items, .. } = response.outcome else {
            panic!("expected result");
        };
        assert_eq!(
            items[0].view,
            View::Pair {
                source: request.query,
                result: "1830".into()
            }
        );
    }

    #[test]
    fn zero_limit_does_not_evaluate_a_pathological_expression() {
        let mut request = request("2^1000000000");
        request.limit = 0;
        let bytes = handle_with(&request.encode().unwrap(), |_| panic!("must not evaluate")).unwrap();
        let response = Response::decode(&bytes, &request).unwrap();
        assert!(matches!(response.outcome, Outcome::Results { items, .. } if items.is_empty()));
    }

    #[test]
    fn unknown_commands_have_an_explicit_error() {
        let mut request = request("2+2");
        request.command = "calendar".into();
        let bytes = handle(&request.encode().unwrap()).unwrap();
        let response = Response::decode(&bytes, &request).unwrap();
        assert!(matches!(
            response.outcome,
            Outcome::Error {
                code: ErrorCode::UnsupportedCommand,
                ..
            }
        ));
    }

    #[test]
    fn unsupported_saved_state_is_not_silently_ignored() {
        let mut request = request("2+2");
        request.state = Some(dupin_plugin_protocol::State {
            version: 1,
            data: Default::default(),
        });
        let bytes = handle_with(&request.encode().unwrap(), |_| panic!("must not evaluate")).unwrap();
        let response = Response::decode(&bytes, &request).unwrap();
        assert!(matches!(
            response.outcome,
            Outcome::Error {
                code: ErrorCode::InvalidQuery,
                ..
            }
        ));
    }

    #[test]
    fn uninteresting_previews_do_not_become_errors() {
        for query in ["42", "not-maths", "2^200"] {
            let request = request(query);
            let bytes = handle(&request.encode().unwrap()).unwrap();
            let response = Response::decode(&bytes, &request).unwrap();
            assert!(matches!(response.outcome, Outcome::Results { items, .. } if items.is_empty()));
        }
    }
}
