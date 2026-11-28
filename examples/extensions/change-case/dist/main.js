// node_modules/change-case/dist/index.js
const SPLIT_LOWER_UPPER_RE = /([\p{Ll}\d])(\p{Lu})/gu;
const SPLIT_UPPER_UPPER_RE = /(\p{Lu})([\p{Lu}][\p{Ll}])/gu;
const SPLIT_SEPARATE_NUMBER_RE = /(\d)\p{Ll}|(\p{L})\d/u;
const DEFAULT_STRIP_REGEXP = /[^\p{L}\d]+/giu;
const SPLIT_REPLACE_VALUE = "$1\0$2";
const DEFAULT_PREFIX_SUFFIX_CHARACTERS = "";
/**
* Split any cased input strings into an array of words.
*/
function split(value) {
	let result = value.trim();
	result = result.replace(SPLIT_LOWER_UPPER_RE, SPLIT_REPLACE_VALUE).replace(SPLIT_UPPER_UPPER_RE, SPLIT_REPLACE_VALUE);
	result = result.replace(DEFAULT_STRIP_REGEXP, "\0");
	let start = 0;
	let end = result.length;
	while (result.charAt(start) === "\0") start++;
	if (start === end) return [];
	while (result.charAt(end - 1) === "\0") end--;
	return result.slice(start, end).split(/\0/g);
}
/**
* Split the input string into an array of words, separating numbers.
*/
function splitSeparateNumbers(value) {
	const words = split(value);
	for (let i = 0; i < words.length; i++) {
		const word = words[i];
		const match = SPLIT_SEPARATE_NUMBER_RE.exec(word);
		if (match) {
			const offset = match.index + (match[1] ?? match[2]).length;
			words.splice(i, 1, word.slice(0, offset), word.slice(offset));
		}
	}
	return words;
}
/**
* Convert a string to space separated lower case (`foo bar`).
*/
function noCase(input, options) {
	const [prefix, words, suffix] = splitPrefixSuffix(input, options);
	return prefix + words.map(lowerFactory(options?.locale)).join(options?.delimiter ?? " ") + suffix;
}
/**
* Convert a string to camel case (`fooBar`).
*/
function camelCase(input, options) {
	const [prefix, words, suffix] = splitPrefixSuffix(input, options);
	const lower = lowerFactory(options?.locale);
	const upper = upperFactory(options?.locale);
	const transform = options?.mergeAmbiguousCharacters ? capitalCaseTransformFactory(lower, upper) : pascalCaseTransformFactory(lower, upper);
	return prefix + words.map((word, index) => {
		if (index === 0) return lower(word);
		return transform(word, index);
	}).join(options?.delimiter ?? "") + suffix;
}
/**
* Convert a string to pascal case (`FooBar`).
*/
function pascalCase(input, options) {
	const [prefix, words, suffix] = splitPrefixSuffix(input, options);
	const lower = lowerFactory(options?.locale);
	const upper = upperFactory(options?.locale);
	const transform = options?.mergeAmbiguousCharacters ? capitalCaseTransformFactory(lower, upper) : pascalCaseTransformFactory(lower, upper);
	return prefix + words.map(transform).join(options?.delimiter ?? "") + suffix;
}
/**
* Convert a string to constant case (`FOO_BAR`).
*/
function constantCase(input, options) {
	const [prefix, words, suffix] = splitPrefixSuffix(input, options);
	return prefix + words.map(upperFactory(options?.locale)).join(options?.delimiter ?? "_") + suffix;
}
/**
* Convert a string to kebab case (`foo-bar`).
*/
function kebabCase(input, options) {
	return noCase(input, {
		delimiter: "-",
		...options
	});
}
/**
* Convert a string to snake case (`foo_bar`).
*/
function snakeCase(input, options) {
	return noCase(input, {
		delimiter: "_",
		...options
	});
}
function lowerFactory(locale) {
	return locale === false ? (input) => input.toLowerCase() : (input) => input.toLocaleLowerCase(locale);
}
function upperFactory(locale) {
	return locale === false ? (input) => input.toUpperCase() : (input) => input.toLocaleUpperCase(locale);
}
function capitalCaseTransformFactory(lower, upper) {
	return (word) => `${upper(word[0])}${lower(word.slice(1))}`;
}
function pascalCaseTransformFactory(lower, upper) {
	return (word, index) => {
		const char0 = word[0];
		return (index > 0 && char0 >= "0" && char0 <= "9" ? "_" + char0 : upper(char0)) + lower(word.slice(1));
	};
}
function splitPrefixSuffix(input, options = {}) {
	const splitFn = options.split ?? (options.separateNumbers ? splitSeparateNumbers : split);
	const prefixCharacters = options.prefixCharacters ?? DEFAULT_PREFIX_SUFFIX_CHARACTERS;
	const suffixCharacters = options.suffixCharacters ?? DEFAULT_PREFIX_SUFFIX_CHARACTERS;
	let prefixIndex = 0;
	let suffixIndex = input.length;
	while (prefixIndex < input.length) {
		const char = input.charAt(prefixIndex);
		if (!prefixCharacters.includes(char)) break;
		prefixIndex++;
	}
	while (suffixIndex > prefixIndex) {
		const index = suffixIndex - 1;
		const char = input.charAt(index);
		if (!suffixCharacters.includes(char)) break;
		suffixIndex = index;
	}
	return [
		input.slice(0, prefixIndex),
		splitFn(input.slice(prefixIndex, suffixIndex)),
		input.slice(suffixIndex)
	];
}
// src/main.ts
const CASES = [
	["camelCase", camelCase],
	["PascalCase", pascalCase],
	["snake_case", snakeCase],
	["kebab-case", kebabCase],
	["CONSTANT_CASE", constantCase]
];
const MAX_TITLE_BYTES = 512;
const MAX_TEXT_BYTES = 16384;
function search(query, _ctx) {
	const input = query.trim();
	if (!input) return [];
	return CASES.flatMap(([name, convert]) => {
		const converted = convert(input);
		if (!converted || utf8Length(converted) > MAX_TEXT_BYTES) return [];
		return [{
			title: clip(converted, MAX_TITLE_BYTES),
			subtitle: name,
			text: converted
		}];
	});
}
function utf8Length(text) {
	let bytes = 0;
	for (const char of text) bytes += charBytes(char);
	return bytes;
}
function charBytes(char) {
	const point = char.codePointAt(0) ?? 0;
	return point < 128 ? 1 : point < 2048 ? 2 : point < 65536 ? 3 : 4;
}
/** `text`, or as much of it as fits in `limit` bytes with an ellipsis. */
function clip(text, limit) {
	if (utf8Length(text) <= limit) return text;
	const ellipsis = "…";
	let budget = limit - charBytes(ellipsis);
	let clipped = "";
	for (const char of text) {
		budget -= charBytes(char);
		if (budget < 0) break;
		clipped += char;
	}
	return clipped + ellipsis;
}
export { search };
