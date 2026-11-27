//! A fixed clock and random seed for one call, so a fixture replays to the same answer on every
//! run and every machine.

use std::cell::RefCell;

use rquickjs::{Ctx, Function, Module, WriteOptions};
use serde::{Deserialize, Serialize};

/// The furthest from the epoch a JavaScript `Date` reaches, in milliseconds.
pub const MAX_TIME: i64 = 8_640_000_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// Milliseconds since the Unix epoch that `Date.now()` and `new Date()` report, unmoving.
    pub now: i64,
    /// Seeds `Math.random`.
    pub seed: u32,
}

impl Environment {
    pub fn is_valid(&self) -> bool {
        (-MAX_TIME..=MAX_TIME).contains(&self.now)
    }
}

/// Replaces `Date` with one whose clock stands at `now`, sharing the real prototype so
/// `instanceof Date` and every method still work, and `Math.random` with mulberry32 from `seed`.
const PRELUDE: &str = r#"export default function (fixed, seed) {
	'use strict';
	const RealDate = Date;
	const construct = Reflect.construct;
	function FixedDate(...args) {
		if (new.target === undefined) return new RealDate(fixed).toString();
		return construct(RealDate, args.length === 0 ? [fixed] : args, new.target);
	}
	const method = (value) => ({ value, writable: true, configurable: true });
	Object.defineProperties(FixedDate, {
		prototype: { value: RealDate.prototype },
		now: method(function now() {
			return fixed;
		}),
		parse: method(RealDate.parse),
		UTC: method(RealDate.UTC),
		length: { value: RealDate.length },
		name: { value: 'Date' },
	});
	Object.defineProperty(RealDate.prototype, 'constructor', method(FixedDate));
	Object.defineProperty(globalThis, 'Date', method(FixedDate));
	let state = seed >>> 0;
	Object.defineProperty(
		Math,
		'random',
		method(function random() {
			state = (state + 0x6d2b79f5) >>> 0;
			let t = state;
			t = Math.imul(t ^ (t >>> 15), t | 1);
			t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
			return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
		}),
	);
}"#;

/// A name no package module can have, since theirs are relative paths.
const PRELUDE_MODULE: &str = "dupin:environment";

thread_local! {
    /// The prelude compiled once for the calls this thread runs: parsing it again for each call
    /// took about as long as a short query.
    static PRELUDE_BYTECODE: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

pub(crate) fn install(ctx: &Ctx<'_>, environment: &Environment) -> rquickjs::Result<()> {
    let compiled = PRELUDE_BYTECODE.with_borrow(Clone::clone);
    let module = match compiled {
        // SAFETY: this thread wrote the bytes from the prelude it had just parsed.
        Some(bytecode) => unsafe { Module::load(ctx.clone(), &bytecode)? },
        None => {
            let module = Module::declare(ctx.clone(), PRELUDE_MODULE, PRELUDE)?;
            let bytecode = module.write(WriteOptions::default())?;
            PRELUDE_BYTECODE.with_borrow_mut(|kept| *kept = Some(bytecode));
            module
        }
    };
    let (module, evaluated) = module.eval()?;
    evaluated.finish::<()>()?;
    let prelude: Function = module.get("default")?;
    prelude.call((environment.now as f64, environment.seed))
}
