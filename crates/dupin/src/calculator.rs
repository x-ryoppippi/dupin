use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use dupin_core::{Action, Item, ItemAction, Provider, ProviderError, Request, View};
use fend_core::{Context, ExchangeRateFnV2, ExchangeRateFnV2Options, Interrupt, SpanKind};

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

const MAX_QUERY_BYTES: usize = 256;
const MAX_PARENTHESIS_DEPTH: usize = 16;
const MAX_ANSWER_BYTES: usize = 256;
const EVALUATION_BUDGET: Duration = Duration::from_millis(50);

#[derive(Default)]
pub(super) struct Calculator {
    currency: Arc<currency::State>,
}

#[derive(Debug, PartialEq)]
enum Answer {
    Local(String),
    Currency { amount: String, caption: String },
    RatesUnavailable,
}

impl Provider for Calculator {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if request.limit == 0 || request.query.len() > MAX_QUERY_BYTES {
            return Ok(());
        }
        let source = request.query.trim();
        let budget = Budget {
            request,
            deadline: Instant::now() + EVALUATION_BUDGET,
        };
        let currency_enabled = self.currency.enabled();
        let rates = currency_enabled.then(|| self.currency.snapshot()).flatten();
        if let Some(answer) = calculate(source, &budget, currency_enabled, rates.as_ref()) {
            let (result, copy) = match answer {
                Answer::Local(result) => (result.clone(), Some(result)),
                Answer::Currency { amount, caption } => (format!("{amount}\n{caption}"), Some(amount)),
                Answer::RatesUnavailable => ("Exchange rates unavailable".into(), None),
            };
            emit(vec![Item {
                id: "answer".into(),
                view: View::Pair {
                    source: source.into(),
                    result,
                },
                record_usage: false,
                icon: None,
                matched: Vec::new(),
                completion: None,
                score: None,
                primary: copy.as_ref().map(|_| "copy".into()),
                alternate: None,
                actions: copy
                    .into_iter()
                    .map(|value| ItemAction::new("copy", "Copy result", Action::Copy(value)))
                    .collect(),
            }]);
        }
        Ok(())
    }
}

struct Budget<'a> {
    request: &'a Request,
    deadline: Instant,
}

impl Interrupt for Budget<'_> {
    fn should_interrupt(&self) -> bool {
        self.request.is_cancelled() || Instant::now() >= self.deadline
    }
}

fn calculate(
    source: &str,
    budget: &impl Interrupt,
    currency_enabled: bool,
    snapshot: Option<&currency::Snapshot>,
) -> Option<Answer> {
    let bare_amount = currency_enabled && could_be_currency_amount(source);
    if budget.should_interrupt() || !is_expression(source, currency_enabled, bare_amount) {
        return None;
    }
    let input = temperature_input(source);
    let input = euro_input(input);
    let mut context = Context::new();
    let used = Arc::new(AtomicBool::new(false));
    let missing = Arc::new(AtomicBool::new(false));
    if currency_enabled {
        context.set_exchange_rate_handler_v2(RateLookup {
            rates: snapshot.map(|snapshot| snapshot.rates.clone()),
            used: used.clone(),
            missing: missing.clone(),
        });
    }
    let result = evaluate(&input, &mut context, budget)?;
    if !used.load(Ordering::Relaxed) {
        return is_expression(source, false, false).then(|| Answer::Local(result.get_main_result().into()));
    }
    if missing.load(Ordering::Relaxed) || snapshot.is_none() {
        return Some(Answer::RatesUnavailable);
    }
    let snapshot = snapshot?;
    let amount = if let Some(code) = result_currency(&result, &snapshot.rates) {
        let explicit = source
            .split_whitespace()
            .any(|word| matches!(word, "to" | "in" | "as"));
        let target = if explicit { code.as_str() } else { "EUR" };
        let precision = if target == "JPY" { 0 } else { 2 };
        let rounded = evaluate(
            &format!("({input}) to {target} to {precision}dp"),
            &mut context,
            budget,
        )?;
        let number = rounded
            .get_main_result_spans()
            .find(|span| span.kind() == SpanKind::Number)?
            .string();
        let grouped = group_digits(number);
        match target {
            "EUR" => format!("≈ €{grouped}"),
            "GBP" => format!("≈ £{grouped}"),
            "JPY" => format!("≈ ¥{grouped}"),
            "USD" => format!("≈ ${grouped}"),
            _ => format!("≈ {grouped} {target}"),
        }
    } else {
        result.get_main_result().into()
    };
    let status = if snapshot.is_stale() {
        "Cached ECB rate"
    } else {
        "ECB rate"
    };
    Some(Answer::Currency {
        amount,
        caption: format!("{status} {}", snapshot.rates.date),
    })
}

struct RateLookup {
    rates: Option<Arc<currency::Rates>>,
    used: Arc<AtomicBool>,
    missing: Arc<AtomicBool>,
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
        // Allow fend to validate the expression. The caller discards any result using a placeholder.
        self.missing.store(true, Ordering::Relaxed);
        Ok(1.0)
    }
}

fn evaluate(input: &str, context: &mut Context, budget: &impl Interrupt) -> Option<fend_core::FendResult> {
    let result = fend_core::evaluate_with_interrupt(input, context, budget).ok()?;
    let answer = result.get_main_result();
    (!budget.should_interrupt()
        && answer.len() <= MAX_ANSWER_BYTES
        && result
            .get_main_result_spans()
            .any(|span| span.kind() == SpanKind::Number))
    .then_some(result)
}

fn result_currency(result: &fend_core::FendResult, rates: &currency::Rates) -> Option<String> {
    let mut parts = result
        .get_main_result_spans()
        .filter(|span| span.string() != "approx. ");
    let (first, second) = (parts.next()?, parts.next()?);
    let symbol = match (first.kind(), second.kind(), parts.next()) {
        (SpanKind::Ident, SpanKind::Number, None) => first.string().trim(),
        (SpanKind::Number, SpanKind::Ident, None) => second.string().trim(),
        _ => return None,
    };
    let code = match symbol {
        "€" => "EUR",
        "£" => "GBP",
        "¥" => "JPY",
        "$" | "US$" | "dollar" | "dollars" => "USD",
        "euro" | "euros" => "EUR",
        "AU$" => "AUD",
        "HK$" => "HKD",
        "NZ$" => "NZD",
        "zł" | "zl" => "PLN",
        "₺" | "TL" => "TRY",
        _ => symbol,
    }
    .to_ascii_uppercase();
    rates.values.contains_key(&code).then_some(code)
}

fn group_digits(number: &str) -> String {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let digits = whole.trim_start_matches('-');
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return number.into();
    }
    let mut grouped = String::new();
    if whole.starts_with('-') {
        grouped.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if index != 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if !fraction.is_empty() {
        grouped.push('.');
        grouped.push_str(fraction);
    }
    grouped
}

fn could_be_currency_amount(source: &str) -> bool {
    source.contains(['$', '€', '£', '¥', '₺'])
        || source.chars().rev().take_while(char::is_ascii_alphabetic).count() == 3
}

fn euro_input(input: Cow<'_, str>) -> Cow<'_, str> {
    if !input.contains('€') {
        return input;
    }
    // Fend's lexer treats a leading euro sign as an identifier, while it accepts 50 EUR.
    let mut formatted = String::new();
    for (index, ch) in input.char_indices() {
        if ch == '€'
            && input[index + ch.len_utf8()..]
                .trim_start()
                .starts_with(|ch: char| ch.is_ascii_digit() || ch == '.')
        {
            formatted.push_str("EUR * ");
        } else {
            formatted.push(ch);
        }
    }
    Cow::Owned(formatted)
}

fn is_expression(source: &str, currency_enabled: bool, bare_amount: bool) -> bool {
    if source.len() > MAX_QUERY_BYTES
        || !source.bytes().any(|byte| byte.is_ascii_digit())
        || source.parse::<f64>().is_ok()
    {
        return false;
    }
    // Fend accepts unclosed parentheses and its left-shift loop ignores interrupts.
    let mut depth = 0usize;
    for ch in source.chars() {
        match ch {
            '(' if depth < MAX_PARENTHESIS_DEPTH => depth += 1,
            ')' if depth > 0 => depth -= 1,
            '+' | '-' | '−' | '*' | '×' | '/' | '÷' | '∕' | '^' | '%' | '!' | '.' | ',' | '_' | '°' | ' ' =>
                {}
            '$' | '€' | '£' | '¥' | '₺' if currency_enabled => {}
            ch if ch.is_ascii_digit() || (ch.is_alphabetic() && ch != 'λ') => {}
            _ => return false,
        }
    }
    let unsigned = source.trim_start_matches(['+', '-', '−']);
    depth == 0
        && (bare_amount
            || unsigned.contains(['+', '-', '−', '*', '×', '/', '÷', '∕', '^', '%', '!', '('])
            || source
                .split_whitespace()
                .any(|word| matches!(word, "to" | "in" | "as" | "mod")))
}

fn temperature_input(source: &str) -> Cow<'_, str> {
    let words: Vec<_> = source.split_whitespace().collect();
    // Lowercase f and c have other meanings outside an explicit temperature conversion.
    match words.as_slice() {
        [
            value @ ..,
            from @ ("f" | "F" | "°F" | "c" | "C" | "°C"),
            connector @ ("to" | "in" | "as"),
            to @ ("f" | "F" | "°F" | "c" | "C" | "°C"),
        ] => Cow::Owned(format!(
            "{} {} {connector} {}",
            value.join(" "),
            from.to_ascii_uppercase(),
            to.to_ascii_uppercase()
        )),
        _ => Cow::Borrowed(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn currency_answer(query: &str, snapshot: Option<&currency::Snapshot>) -> Option<Answer> {
        let request = Request::detached(query, 1);
        let budget = Budget {
            request: &request,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        calculate(query, &budget, true, snapshot)
    }

    #[test]
    fn converts_currencies_with_the_observation_date() {
        let snapshot = currency::fixture();
        for (query, amount) in [
            ("100 usd to jpy", "≈ ¥14,950"),
            ("€50 in gbp", "≈ £42.5"),
            ("¥5000 in gbp", "≈ £25.84"),
            ("30*(60+1) usd in jpy", "≈ ¥273,585"),
            ("10 GBP", "≈ €11.76"),
            ("-10 GBP", "≈ €-11.76"),
            ("100 usd to CAD", "≈ 131.82 CAD"),
        ] {
            assert_eq!(
                currency_answer(query, Some(&snapshot)),
                Some(Answer::Currency {
                    amount: amount.into(),
                    caption: "ECB rate 2026-11-20".into()
                }),
                "{query}"
            );
        }
        let stale = currency::Snapshot {
            refreshed: std::time::SystemTime::UNIX_EPOCH,
            ..snapshot
        };
        assert_eq!(
            currency_answer("100 usd to jpy", Some(&stale)),
            Some(Answer::Currency {
                amount: "≈ ¥14,950".into(),
                caption: "Cached ECB rate 2026-11-20".into()
            })
        );
    }

    #[test]
    fn reports_missing_rates_without_claiming_invalid_queries() {
        let snapshot = currency::fixture();
        for query in ["100 usd to jpy", "€50 in gbp", "10 gbp"] {
            assert_eq!(
                currency_answer(query, None),
                Some(Answer::RatesUnavailable),
                "{query}"
            );
        }
        assert_eq!(
            currency_answer("100 usd to CHF", Some(&snapshot)),
            Some(Answer::RatesUnavailable)
        );
        for query in [
            "100 usd to",
            "€50 in missing",
            "(10 GBP",
            "5 min",
            "app123",
            "1Password",
        ] {
            assert_eq!(currency_answer(query, None), None, "{query}");
        }
        assert_eq!(
            currency_answer("1 gb to tb", None),
            Some(Answer::Local("0.001 Tb".into()))
        );
    }

    fn answer(source: &str) -> Option<String> {
        let request = Request::detached(source, 1);
        let budget = Budget {
            request: &request,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        match calculate(source, &budget, false, None)? {
            Answer::Local(result) => Some(result),
            _ => None,
        }
    }

    #[test]
    fn evaluates_arithmetic_and_units() {
        for (query, expected) in [
            ("12*3", "36"),
            ("2^10", "1024"),
            ("(1+2)/3", "1"),
            ("10 mod 3", "1"),
            ("15% of 200", "30"),
            ("5 km to mi", "approx. 3.1068559612 mi"),
            ("5 miles to feet", "26400 feet"),
            ("100 f to c", "approx. 37.7777777778 °C"),
            ("(20+12) °F in c", "0 °C"),
            ("1 GiB in MB", "1073.741824 MB"),
            ("2 m^2 in cm^2", "20000 cm^2"),
            ("30*(60+1)km/h in m/s", "approx. 508.3333333333 m / s"),
        ] {
            assert_eq!(answer(query).as_deref(), Some(expected), "{query}");
        }
    }

    #[test]
    fn skips_non_expressions_and_incomplete_queries() {
        for query in [
            "",
            "Calculator",
            "square foot",
            "pi",
            "m/s",
            "42",
            "-42",
            "1e-3",
            "0xff",
            "-0xff",
            "1Password",
            "C++17",
            "12*",
            "(1+2",
            "(1+2))",
            "5 km to",
            "1/0",
            "2+2\n5",
            "1<<1000000000000",
            "x=1; x+2",
            "100 usd to jpy",
        ] {
            assert_eq!(answer(query), None, "{query}");
        }
        assert_eq!(answer(&format!("{}+1", "1".repeat(MAX_QUERY_BYTES))), None);
        let depth = MAX_PARENTHESIS_DEPTH + 1;
        assert_eq!(
            answer(&format!("{}1+2{}", "(".repeat(depth), ")".repeat(depth))),
            None
        );
        assert_eq!(answer("2^1000"), None);
    }

    #[test]
    fn expensive_work_is_interrupted() {
        let request = Request::detached("2^2^40", 1);
        let start = Instant::now();
        let mut batches = Vec::new();
        Calculator::default()
            .search(&request, &mut |batch| batches.extend(batch))
            .unwrap();
        assert!(batches.is_empty());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
