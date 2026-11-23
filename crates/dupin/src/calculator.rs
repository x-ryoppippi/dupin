use std::borrow::Cow;
use std::time::{Duration, Instant};

use dupin_core::{Action, Item, ItemAction, Provider, ProviderError, Request, View};
use fend_core::{Context, Interrupt, SpanKind};

const MAX_QUERY_BYTES: usize = 256;
const MAX_PARENTHESIS_DEPTH: usize = 16;
const MAX_ANSWER_BYTES: usize = 256;
const EVALUATION_BUDGET: Duration = Duration::from_millis(50);

pub(super) struct Calculator;

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
        if let Some(result) = calculate(source, &budget) {
            emit(vec![Item {
                id: "answer".into(),
                view: View::Pair {
                    source: source.into(),
                    result: result.clone(),
                },
                record_usage: false,
                icon: None,
                matched: Vec::new(),
                completion: None,
                score: None,
                primary: Some("copy".into()),
                alternate: None,
                actions: vec![ItemAction::new("copy", "Copy result", Action::Copy(result))],
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

fn calculate(source: &str, budget: &impl Interrupt) -> Option<String> {
    if budget.should_interrupt() || !is_expression(source) {
        return None;
    }
    let input = temperature_input(source);
    let result = fend_core::evaluate_with_interrupt(&input, &mut Context::new(), budget).ok()?;
    let answer = result.get_main_result();
    (!budget.should_interrupt()
        && answer.len() <= MAX_ANSWER_BYTES
        && result
            .get_main_result_spans()
            .any(|span| span.kind() == SpanKind::Number))
    .then(|| answer.to_owned())
}

fn is_expression(source: &str) -> bool {
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
            ch if ch.is_ascii_digit() || (ch.is_alphabetic() && ch != 'λ') => {}
            _ => return false,
        }
    }
    let unsigned = source.trim_start_matches(['+', '-', '−']);
    depth == 0
        && (unsigned.contains(['+', '-', '−', '*', '×', '/', '÷', '∕', '^', '%', '!', '('])
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

    fn answer(source: &str) -> Option<String> {
        let request = Request::detached(source, 1);
        let budget = Budget {
            request: &request,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        calculate(source, &budget)
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
        Calculator
            .search(&request, &mut |batch| batches.extend(batch))
            .unwrap();
        assert!(batches.is_empty());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
