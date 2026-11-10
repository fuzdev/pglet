/**
 * The programs a release runs, and the two ways it runs them: captured, for
 * questions, and streamed, for long or interactive steps. Neither throws for a
 * failed or missing program; callers decide what a failure means.
 *
 * @module
 */

import type { FsReadDir } from '@fuzdev/gro/fs.ts';
import type { SpawnProgram, SpawnResult } from '@fuzdev/gro/spawn.ts';

/** The outcome of a release step that can fail: its value's fields on success, a message naming the failure otherwise. */
export type ReleaseResult<TValue extends object = object> =
	({ ok: true } & TValue) | { ok: false; error: string };

/** A program a release runs, as the task root found it. */
export interface ReleaseProgram {
	/** The name the program is called by, in messages. */
	name: string;
	/** The program, `null` when it isn't installed. */
	program: SpawnProgram | null;
	/** Where it runs unless a call names another dir: the checkout. */
	cwd: FsReadDir;
}

/** Where one call runs, when not the program's own `cwd`. */
export interface ReleaseCallOptions {
	cwd?: FsReadDir;
}

/** A captured call's outcome. */
export interface ReleaseCapture {
	success: boolean;
	/** Stdout with trailing whitespace trimmed; leading space is kept, since porcelain status lines open with it. */
	out: string;
	/** Stderr, trimmed. */
	err: string;
}

/**
 * Runs `program` and captures its outcome and output. A missing program is a
 * failure whose `err` names it. Stdin is the terminal's, so a call that prompts
 * (ssh, a credential helper, a pinentry) can still reach the user.
 */
export const release_capture = async (
	program: ReleaseProgram,
	args: ReadonlyArray<string>,
	options?: ReleaseCallOptions
): Promise<ReleaseCapture> => {
	if (program.program === null) {
		return { success: false, out: '', err: `${program.name}: command not found` };
	}
	const output = await program.program.out(args, {
		cwd: options?.cwd ?? program.cwd,
		stdin: 'inherit'
	});
	return {
		success: output.kind === 'exited' && output.code === 0,
		out: output.stdout.trimEnd(),
		err: output.stderr.trim()
	};
};

/** A streamed call's outcome. */
export interface ReleaseStream {
	success: boolean;
	/** How it ended, for messages: `exit <code>`, or the signal that ended it. */
	ended: string;
}

/**
 * Runs `program` with the terminal attached, stdin included, so an interactive
 * prompt such as npm's browser auth reaches the user. A missing program prints
 * that it isn't found and ends as exit 127, as a shell would.
 */
export const release_stream = async (
	program: ReleaseProgram,
	args: ReadonlyArray<string>,
	print_error: (text: string) => void,
	options?: ReleaseCallOptions
): Promise<ReleaseStream> => {
	if (program.program === null) {
		print_error(`${program.name}: command not found`);
		return { success: false, ended: 'exit 127' };
	}
	const result = await program.program.run(args, { cwd: options?.cwd ?? program.cwd });
	return { success: result.kind === 'exited' && result.code === 0, ended: to_ended(result) };
};

const to_ended = (result: SpawnResult): string => {
	switch (result.kind) {
		case 'exited':
			return `exit ${result.code}`;
		case 'signaled':
			return result.signal;
		default:
			return result.kind;
	}
};
