/** CSI and OSC escape sequences, and two-character escapes, which a terminal would interpret. */
const ESCAPES = /\u001b(?:\[[0-?]*[ -/]*[@-~]|\][^\u0007\u001b]*(?:\u0007|\u001b\\)?|[@-Z\\-_])/g;

/** Formatting characters that are invisible or reorder text, so they could disguise a row. */
function invisible(code: number): boolean {
	return (
		code === 0x00ad ||
		code === 0x061c ||
		(code >= 0x200b && code <= 0x200f) ||
		(code >= 0x2028 && code <= 0x202e) ||
		(code >= 0x2060 && code <= 0x206f) ||
		code === 0xfeff ||
		(code >= 0xfff9 && code <= 0xfffb) ||
		(code >= 0xe0000 && code <= 0xe007f)
	);
}

/**
 * Text a remote machine printed, as plain characters: escape sequences, control characters
 * (tabs become spaces) and invisible or reordering formatting characters are dropped, and an
 * unpaired surrogate becomes U+FFFD. Newlines survive only with `lines`.
 */
export function plain(text: string, lines = false): string {
	let out = '';
	for (const char of text.replace(ESCAPES, '')) {
		const code = char.codePointAt(0) ?? 0;
		if (code === 0x09) out += ' ';
		else if (code === 0x0a && lines) out += '\n';
		else if (code < 0x20 || (code >= 0x7f && code < 0xa0) || invisible(code)) continue;
		else if (code >= 0xd800 && code <= 0xdfff) out += '�';
		else out += char;
	}
	return out;
}

function bytesOf(char: string): number {
	const code = char.codePointAt(0) ?? 0;
	return code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
}

export function utf8Bytes(text: string): number {
	let bytes = 0;
	for (const char of text) bytes += bytesOf(char);
	return bytes;
}

/** `text` cut to at most `maxBytes` of UTF-8, ending with an ellipsis when it was cut. */
export function clamp(text: string, maxBytes: number): string {
	if (utf8Bytes(text) <= maxBytes) return text;
	let out = '';
	let bytes = 0;
	for (const char of text) {
		const size = bytesOf(char);
		if (bytes + size > maxBytes - 3) break;
		out += char;
		bytes += size;
	}
	return `${out}…`;
}

/** One line for a title, subtitle or label: plain text, whitespace collapsed and cut to fit. */
export function line(text: string, maxBytes: number): string {
	return clamp(plain(text).replace(/\s+/g, ' ').trim(), maxBytes);
}

/** `body` as a fenced code block that no backtick run inside it can close. */
export function fenced(body: string, maxBytes: number): string {
	const text = clamp(plain(body, true), maxBytes);
	let longest = 0;
	for (const run of text.match(/`+/g) ?? []) longest = Math.max(longest, run.length);
	const fence = '`'.repeat(Math.max(3, longest + 1));
	return `${fence}\n${text}\n${fence}`;
}

/** How long ago `then` was, for a page Dupin doesn't date itself, as Dupin writes it. */
export function ago(now: number, then: number): string {
	const seconds = Math.max(0, Math.floor((now - then) / 1000));
	if (seconds < 60) return 'Just now';
	if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
	if (seconds < 86_400) return `${Math.floor(seconds / 3600)} h ago`;
	return `${Math.floor(seconds / 86_400)} d ago`;
}
