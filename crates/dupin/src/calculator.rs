//! The `calculator` service: fend works an expression out for the bundled Calculator, with the
//! European Central Bank's reference rates while currency conversion is on. The package decides
//! what counts as a calculation and how its answer reads; Dupin keeps what fend may be handed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use dupin_extension_bridge::Failure;
use dupin_extension_bridge::services;
use fend_core::{Context, ExchangeRateFnV2, ExchangeRateFnV2Options, Interrupt, SpanKind};
use serde_json::{Value, json};

mod currency;

pub(super) struct Currency(currency::Worker);

impl Currency {
    pub fn start(notify: impl Fn() + Send + 'static) -> Self {
        Self(currency::Worker::start(notify))
    }

    pub fn calculator(&self) -> Calculator {
        Calculator {
            currency: self.0.state.clone(),
        }
    }

    pub fn set_enabled(&self, enabled: bool) -> bool {
        self.0.set_enabled(enabled)
    }

    pub fn enabled(&self) -> bool {
        self.0.state.enabled()
    }
}

const MAX_EXPRESSION_BYTES: usize = 512;
// fend recurses into each bracket. The package nests a conversion one level deeper than what
// was typed, which it allows 16 levels.
const MAX_PARENTHESIS_DEPTH: usize = 17;
const MAX_ANSWER_BYTES: usize = 256;
const EVALUATION_BUDGET: Duration = Duration::from_millis(50);

#[derive(Clone, Default)]
pub(crate) struct Calculator {
    currency: Arc<currency::State>,
}

impl services::Calculator for Calculator {
    fn evaluate(&self, expression: &str, cancelled: &AtomicBool) -> Result<Value, Failure> {
        let budget = Budget {
            cancelled,
            deadline: Instant::now() + EVALUATION_BUDGET,
        };
        let currency = self.currency.enabled();
        let snapshot = currency.then(|| self.currency.snapshot()).flatten();
        Ok(evaluation(expression, &budget, currency, snapshot.as_ref()).unwrap_or(Value::Null))
    }
}

struct Budget<'a> {
    cancelled: &'a AtomicBool,
    deadline: Instant,
}

impl Interrupt for Budget<'_> {
    fn should_interrupt(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed) || Instant::now() >= self.deadline
    }
}

fn evaluation(
    expression: &str,
    budget: &impl Interrupt,
    currency: bool,
    snapshot: Option<&currency::Snapshot>,
) -> Option<Value> {
    if budget.should_interrupt() || !admissible(expression) {
        return None;
    }
    let mut context = Context::new();
    let used = Arc::new(AtomicBool::new(false));
    let missing = Arc::new(OnceLock::new());
    if currency {
        context.set_exchange_rate_handler_v2(RateLookup {
            rates: snapshot.map(|snapshot| snapshot.rates.clone()),
            used: used.clone(),
            missing: missing.clone(),
        });
    }
    let result = fend_core::evaluate_with_interrupt(expression, &mut context, budget).ok()?;
    if budget.should_interrupt() || result.get_main_result().len() > MAX_ANSWER_BYTES {
        return None;
    }
    let number = result
        .get_main_result_spans()
        .find(|span| span.kind() == SpanKind::Number)?
        .string()
        .to_owned();
    let parts: Vec<_> = result
        .get_main_result_spans()
        .map(|span| {
            let kind = match span.kind() {
                SpanKind::Number => "number",
                SpanKind::Ident => "ident",
                _ => "other",
            };
            json!({ "kind": kind, "text": span.string() })
        })
        .collect();
    let mut answer = json!({
        "result": result.get_main_result(),
        "number": number,
        "parts": parts,
    });
    if used.load(Ordering::Relaxed) {
        answer["exchange"] = match (snapshot, missing.get()) {
            (None, _) => json!({ "status": "unavailable" }),
            (Some(_), Some(code)) => json!({ "status": "missing", "currency": code }),
            (Some(snapshot), None) => {
                let mut currencies: Vec<_> = snapshot.rates.values.keys().collect();
                currencies.sort();
                json!({
                    "status": "ok",
                    "date": snapshot.rates.date,
                    "stale": snapshot.is_stale(),
                    "currencies": currencies,
                })
            }
        };
    }
    Some(answer)
}

/// Whether fend may be handed `expression`: it accepts unclosed brackets, recurses into each
/// one, and its left shift ignores interrupts, so only balanced, shallow arithmetic, units and
/// currencies reach it, whatever the package asks.
fn admissible(expression: &str) -> bool {
    if expression.len() > MAX_EXPRESSION_BYTES {
        return false;
    }
    let mut depth = 0usize;
    for ch in expression.chars() {
        match ch {
            '(' if depth < MAX_PARENTHESIS_DEPTH => depth += 1,
            ')' if depth > 0 => depth -= 1,
            '+' | '-' | '−' | '*' | '×' | '/' | '÷' | '∕' | '^' | '%' | '!' | '.' | ',' | '_' | '°' | ' ' =>
                {}
            '$' | '€' | '£' | '¥' | '₺' => {}
            ch if ch.is_ascii_digit() || (ch.is_alphabetic() && ch != 'λ') => {}
            _ => return false,
        }
    }
    depth == 0
}

struct RateLookup {
    rates: Option<Arc<currency::Rates>>,
    used: Arc<AtomicBool>,
    missing: Arc<OnceLock<String>>,
}

impl ExchangeRateFnV2 for RateLookup {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _: &ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        self.used.store(true, Ordering::Relaxed);
        if let Some(rate) = self.rates.as_ref().and_then(|rates| rates.values.get(currency)) {
            return Ok(*rate);
        }
        // Allow fend to validate the expression. The answer reports the missing rate instead.
        self.missing.get_or_init(|| currency.to_owned());
        Ok(1.0)
    }
}

#[cfg(test)]
mod tests {
    use services::Calculator as _;

    use super::*;

    fn evaluate(calculator: &Calculator, expression: &str) -> Value {
        calculator.evaluate(expression, &AtomicBool::new(false)).unwrap()
    }

    fn result(expression: &str) -> Option<String> {
        let answer = evaluate(&Calculator::default(), expression);
        answer["result"].as_str().map(str::to_owned)
    }

    #[test]
    fn evaluates_arithmetic_and_units() {
        for (expression, expected) in [
            ("12*3", "36"),
            ("2^10", "1024"),
            ("(1+2)/3", "1"),
            ("10 mod 3", "1"),
            ("15% of 200", "30"),
            ("5 km to mi", "approx. 3.1068559612 mi"),
            ("5 miles to feet", "26400 feet"),
            ("100 F to C", "approx. 37.7777777778 °C"),
            ("(20+12) °F in C", "0 °C"),
            ("1 GiB in MB", "1073.741824 MB"),
            ("2 m^2 in cm^2", "20000 cm^2"),
            ("30*(60+1)km/h in m/s", "approx. 508.3333333333 m / s"),
        ] {
            assert_eq!(result(expression).as_deref(), Some(expected), "{expression}");
        }
        assert_eq!(
            evaluate(&Calculator::default(), "5 miles to feet"),
            json!({
                "result": "26400 feet",
                "number": "26400",
                "parts": [{ "kind": "number", "text": "26400" }, { "kind": "ident", "text": " feet" }],
            })
        );
    }

    #[test]
    fn refuses_what_fend_must_not_be_handed() {
        for expression in [
            "1<<1000000000000",
            "x=1; x+2",
            "(1+2",
            "(1+2))",
            "2+2\n5",
            "λx.x",
            "1/0",
            "2^1000",
        ] {
            assert_eq!(result(expression), None, "{expression}");
        }
        assert_eq!(result(&format!("{}+1", "1".repeat(MAX_EXPRESSION_BYTES))), None);
        let deep = |depth| format!("{}1+2{}", "(".repeat(depth), ")".repeat(depth));
        assert_eq!(result(&deep(MAX_PARENTHESIS_DEPTH)).as_deref(), Some("3"));
        assert_eq!(result(&deep(MAX_PARENTHESIS_DEPTH + 1)), None);
    }

    #[test]
    fn expensive_work_is_interrupted() {
        let start = Instant::now();
        assert_eq!(result("2^2^40"), None);
        assert!(start.elapsed() < Duration::from_secs(1));
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            Calculator::default().evaluate("1+1", &cancelled).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn reports_the_rates_a_currency_conversion_used() {
        let with = |snapshot| Calculator {
            currency: Arc::new(currency::State::with_snapshot(snapshot)),
        };
        let current = with(Some(currency::fixture()));
        let answer = evaluate(&current, "(100 usd to jpy) to JPY to 0dp");
        assert_eq!(answer["number"], "14950");
        assert_eq!(answer["exchange"]["status"], "ok");
        assert_eq!(answer["exchange"]["date"], "2026-11-20");
        assert_eq!(answer["exchange"]["stale"], false);
        let currencies = answer["exchange"]["currencies"].as_array().unwrap();
        assert!(currencies.contains(&json!("GBP")) && currencies.contains(&json!("EUR")));
        assert_eq!(
            evaluate(&current, "100 usd to CHF")["exchange"],
            json!({ "status": "missing", "currency": "CHF" })
        );
        assert_eq!(
            evaluate(&with(None), "10 GBP")["exchange"],
            json!({ "status": "unavailable" })
        );
        let stale = with(Some(currency::Snapshot {
            refreshed: std::time::SystemTime::UNIX_EPOCH,
            ..currency::fixture()
        }));
        assert_eq!(evaluate(&stale, "10 GBP")["exchange"]["stale"], true);
        assert!(
            evaluate(&current, "12*3").get("exchange").is_none(),
            "no rate was used"
        );
        assert_eq!(
            result("100 usd to jpy"),
            None,
            "without currency conversion, a currency is no unit"
        );
    }
}
