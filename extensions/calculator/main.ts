import { calculator } from 'dupin';
import type { Calculation, CalculationPart, Item } from 'dupin';

// What Dupin used to answer before the port, in UTF-8 bytes; longer queries are left to apps.
const MAX_QUERY_BYTES = 256;
const MAX_PARENTHESIS_DEPTH = 16;

const OPERATORS = ['+', '-', '−', '*', '×', '/', '÷', '∕', '^', '%', '!', '('];
const SYMBOLS = ['$', '€', '£', '¥', '₺'];
const PUNCTUATION = new Set([
	'+',
	'-',
	'−',
	'*',
	'×',
	'/',
	'÷',
	'∕',
	'^',
	'%',
	'!',
	'.',
	',',
	'_',
	'°',
	' ',
]);
const CONNECTORS = new Set(['to', 'in', 'as']);
const TEMPERATURES = new Set(['f', 'F', '°F', 'c', 'C', '°C']);
const PLAIN_NUMBER = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/;
const LETTER = /^\p{Alphabetic}$/u;

// The currency a symbol or name fend writes stands for.
const CURRENCY_NAMES: Record<string, string> = {
	'€': 'EUR',
	'£': 'GBP',
	'¥': 'JPY',
	$: 'USD',
	US$: 'USD',
	dollar: 'USD',
	dollars: 'USD',
	euro: 'EUR',
	euros: 'EUR',
	AU$: 'AUD',
	HK$: 'HKD',
	NZ$: 'NZD',
	zł: 'PLN',
	zl: 'PLN',
	'₺': 'TRY',
	TL: 'TRY',
};
const SIGNS: Record<string, string> = { EUR: '€', GBP: '£', JPY: '¥', USD: '$' };

type Answer = { display: string; number: string } | { display: string; number?: undefined };

export async function search(query: string): Promise<Item[]> {
	if (bytes(query) > MAX_QUERY_BYTES) return [];
	const source = query.trim();
	const answer = await calculate(source);
	if (!answer) return [];
	const row: Item = { id: 'answer', title: source, answer: answer.display };
	if (answer.number !== undefined) row.actions = [{ title: 'Copy number', copy: answer.number }];
	return [row];
}

async function calculate(source: string): Promise<Answer | null> {
	// Dupin converts currencies only while that is on, so a query that reads as one is worked
	// out either way and kept only as a plain calculation when no rate was used.
	if (!isExpression(source, true, couldBeCurrencyAmount(source))) return null;
	const input = euroInput(temperatureInput(source));
	const result = await calculator.evaluate(input);
	if (!result) return null;
	if (!result.exchange) {
		return isExpression(source, false, false)
			? { display: result.result, number: result.number }
			: null;
	}
	let exchange = result.exchange;
	if (exchange.status !== 'ok') return withoutRate(exchange);
	let amount = result.result;
	let number = result.number;
	const code = resultCurrency(result, exchange.currencies);
	if (code) {
		const explicit = words(source).some((word) => CONNECTORS.has(word));
		const target = explicit ? code : 'EUR';
		const precision = ['ISK', 'JPY', 'KRW'].includes(target) ? 0 : 2;
		const converted = await calculator.evaluate(`(${input}) to ${target} to ${precision}dp`);
		if (!converted) return null;
		// Dupin may have refreshed the rates in between: the date is the converted amount's.
		exchange = converted.exchange ?? exchange;
		if (exchange.status !== 'ok') return withoutRate(exchange);
		number = padded(converted.number, precision);
		const grouped = groupDigits(number);
		const sign = SIGNS[target];
		amount = sign ? `≈ ${sign}${grouped}` : `≈ ${grouped} ${target}`;
	}
	const status = exchange.stale ? 'Cached ECB rate' : 'ECB rate';
	return { display: `${amount}\n${status} ${exchange.date}`, number };
}

function withoutRate(
	exchange: { status: 'unavailable' } | { status: 'missing'; currency: string },
): Answer {
	return exchange.status === 'unavailable'
		? { display: 'Exchange rates unavailable' }
		: { display: `No ECB rate for ${exchange.currency}` };
}

/** Whether `source` reads as a calculation rather than a name, a bare number or half a sum. */
function isExpression(source: string, currency: boolean, bareAmount: boolean): boolean {
	if (bytes(source) > MAX_QUERY_BYTES || !/[0-9]/.test(source) || PLAIN_NUMBER.test(source)) {
		return false;
	}
	let depth = 0;
	for (const char of source) {
		if (char === '(' && depth < MAX_PARENTHESIS_DEPTH) depth += 1;
		else if (char === ')' && depth > 0) depth -= 1;
		else if (PUNCTUATION.has(char)) continue;
		else if (currency && SYMBOLS.includes(char)) continue;
		else if (/[0-9]/.test(char) || (LETTER.test(char) && char !== 'λ')) continue;
		else return false;
	}
	const unsigned = source.replace(/^[+\-−]+/u, '');
	return (
		depth === 0 &&
		(bareAmount ||
			OPERATORS.some((operator) => unsigned.includes(operator)) ||
			words(source).some((word) => CONNECTORS.has(word) || word === 'mod'))
	);
}

function couldBeCurrencyAmount(source: string): boolean {
	return (
		SYMBOLS.some((symbol) => source.includes(symbol)) || /(?:^|[^A-Za-z])[A-Za-z]{3}$/.test(source)
	);
}

/** Lowercase f and c mean other things outside an explicit temperature conversion. */
function temperatureInput(source: string): string {
	const parts = words(source);
	if (parts.length < 3) return source;
	const [from, connector, to] = parts.slice(-3);
	if (!TEMPERATURES.has(from) || !CONNECTORS.has(connector) || !TEMPERATURES.has(to)) return source;
	const value = parts.slice(0, -3).join(' ');
	return `${value} ${asciiUpper(from)} ${connector} ${asciiUpper(to)}`;
}

/** fend reads a leading euro sign as a name, while it takes 50 EUR. */
function euroInput(input: string): string {
	if (!input.includes('€')) return input;
	let formatted = '';
	for (let index = 0; index < input.length; index += 1) {
		const char = input[index];
		formatted += char === '€' && /^\s*[0-9.]/u.test(input.slice(index + 1)) ? 'EUR * ' : char;
	}
	return formatted;
}

/** The currency the answer is an amount of, when it is just one amount and Dupin has its rate. */
function resultCurrency(result: Calculation, currencies: string[]): string | null {
	const parts = result.parts.filter((part) => part.text !== 'approx. ');
	if (parts.length !== 2) return null;
	const symbol = unitOf(parts[0], parts[1]);
	if (symbol === null) return null;
	const code = asciiUpper(Object.hasOwn(CURRENCY_NAMES, symbol) ? CURRENCY_NAMES[symbol] : symbol);
	return currencies.includes(code) ? code : null;
}

function unitOf(first: CalculationPart, second: CalculationPart): string | null {
	if (first.kind === 'ident' && second.kind === 'number') return first.text.trim();
	if (first.kind === 'number' && second.kind === 'ident') return second.text.trim();
	return null;
}

/** Writes out the decimals a currency is quoted to that fend leaves off, as in `42.50`. */
function padded(number: string, precision: number): string {
	const point = number.indexOf('.');
	const whole = point < 0 ? number : number.slice(0, point);
	const fraction = point < 0 ? '' : number.slice(point + 1);
	if (
		precision > fraction.length &&
		/^[0-9]*$/.test(whole.replace(/^-+/, '')) &&
		/^[0-9]*$/.test(fraction)
	) {
		return `${number}${point < 0 ? '.' : ''}${'0'.repeat(precision - fraction.length)}`;
	}
	return number;
}

function groupDigits(number: string): string {
	const point = number.indexOf('.');
	const whole = point < 0 ? number : number.slice(0, point);
	const fraction = point < 0 ? '' : number.slice(point + 1);
	const digits = whole.replace(/^-+/, '');
	if (!/^[0-9]*$/.test(digits)) return number;
	let grouped = whole.startsWith('-') ? '-' : '';
	for (let index = 0; index < digits.length; index += 1) {
		if (index !== 0 && (digits.length - index) % 3 === 0) grouped += ',';
		grouped += digits[index];
	}
	return fraction ? `${grouped}.${fraction}` : grouped;
}

function words(text: string): string[] {
	return text.split(/\s+/u).filter((word) => word !== '');
}

function asciiUpper(text: string): string {
	return text.replace(/[a-z]/g, (char) => char.toUpperCase());
}

function bytes(text: string): number {
	let total = 0;
	for (const char of text) {
		const code = char.codePointAt(0) ?? 0;
		total += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
	}
	return total;
}
