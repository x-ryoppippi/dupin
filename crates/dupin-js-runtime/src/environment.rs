//! A fixed clock and random seed for one call, so a fixture replays to the same answer on every
//! run and every machine.

use rquickjs::{Ctx, Function};
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
const PRELUDE: &str = r#"(function (fixed, seed) {
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
})"#;

pub(crate) fn install(ctx: &Ctx<'_>, environment: &Environment) -> rquickjs::Result<()> {
    let prelude: Function = ctx.eval(PRELUDE)?;
    prelude.call((environment.now as f64, environment.seed))
}
