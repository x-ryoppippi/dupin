/**
 * The command-line coding agents the list looks for, by the name their program, or the script
 * an interpreter runs, is called. Add a name here, in an ejected copy, to list another.
 */
export const AGENTS: readonly string[] = [
	'aider',
	'amp',
	'claude',
	'codex',
	'copilot',
	'crush',
	'cursor-agent',
	'gemini',
	'goose',
	'kimi',
	'opencode',
	'qwen',
];

/** Programs that run an agent written as a script, whose first argument names it. */
export const INTERPRETERS: readonly string[] = ['node', 'bun', 'deno', 'python', 'python3'];
