// Everything passed to `ssh` after the host is joined into one command line that the remote
// login shell parses again, so every value from configuration or a row must be a word that no
// POSIX shell, zsh or fish gives a meaning: none of these allow spaces, quotes, `$`, `` ` ``,
// globs, `~`, `;`, `&`, `|`, redirections or a leading `-` or `=`.

/** An SSH host alias or name, optionally with a user: never an option to `ssh`. */
export const HOST = /^[A-Za-z0-9_][A-Za-z0-9._@-]{0,63}$/;
/** A systemd unit name, as `journalctl -u` takes it. */
export const UNIT = /^[A-Za-z0-9_][A-Za-z0-9_.@:+-]{0,127}$/;
/** An absolute path on the remote machine. */
export const PATH = /^\/[A-Za-z0-9._/@:+,=-]{0,159}$/;

export const MIN_LINES = 1;
export const MAX_LINES = 200;
export const DEFAULT_LINES = 50;
/** The most units, log files and folders each host may list. */
export const MAX_SOURCES = 4;

export function validHost(value: unknown): string | null {
	return typeof value === 'string' && HOST.test(value) ? value : null;
}

export function validPid(value: unknown): string | null {
	const text = typeof value === 'number' ? String(value) : value;
	return typeof text === 'string' && /^[1-9][0-9]{0,9}$/.test(text) && Number(text) <= 4_194_304
		? text
		: null;
}

export const TERMINALS = ['Terminal', 'iTerm'] as const;
export type Terminal = (typeof TERMINALS)[number];

export function validTerminal(value: unknown): Terminal {
	return TERMINALS.find((name) => name === value) ?? 'Terminal';
}
