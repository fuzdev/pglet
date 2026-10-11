/**
 * The `CHANGELOG.md` grammar a release reads and writes: the pending `## Unreleased` section, the
 * `<!-- bump: <level> -->` marker on the line after its heading, and the stamp that turns it into
 * the released version's section. Pure text helpers, no I/O. The grammar is LF-only: a CRLF
 * heading is no heading, so a CRLF changelog fails the release's checks loudly instead of
 * stamping mixed line endings.
 *
 * @module
 */

import type {BumpLevel} from './version.ts';

export const CHANGELOG_PATH = 'CHANGELOG.md';

/** The bump marker, recognized only on the line directly after the `## Unreleased` heading, the
 * position `changelog_stamp` strips it from. `changelog_section` returns the body from the
 * newline after the heading, so the leading `\n` anchors that placement: a marker anywhere else
 * is ignored, and validation cannot accept a marker the stamp would leave behind. */
const UNRELEASED_BUMP_MARKER = /^\n<!-- bump: (patch|minor|major) -->(?:\n|$)/;

/** A heading line's end: the next byte is a `\n`, or there is none. Not a multiline `$`, which
 * also matches before a `\r` and so would admit a CRLF heading. */
const LINE_END = '(?![^\\n])';

const UNRELEASED_HEADING = new RegExp(`^## Unreleased${LINE_END}`, 'm');

/** The heading with its marker on the next line, the two lines `changelog_stamp` replaces, down
 * to the marker's terminator, so a marker the reader accepts is one the stamp strips. */
const UNRELEASED_HEADING_WITH_MARKER = /^## Unreleased\n<!-- bump: (?:patch|minor|major) -->(?:\n|$)/m;

/** What the next cycle starts from: an empty `## Unreleased` declaring a patch. */
const FRESH_UNRELEASED = '## Unreleased\n<!-- bump: patch -->\n\n';

/** The `## <heading>` line, the heading text matched literally. */
const heading_re = (heading: string): RegExp =>
	new RegExp(`^## ${heading.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}${LINE_END}`, 'm');

/** The body of the `## <heading>` section (after the heading line, up to the next `## ` heading
 * or the end), or null when there is no such heading. `heading` is the literal text after `## `:
 * `Unreleased` or a version. */
export const changelog_section = (changelog: string, heading: string): string | null => {
	const match = heading_re(heading).exec(changelog);
	if (!match) return null;
	const after = changelog.slice(match.index + match[0].length);
	const next = after.search(/^## /m);
	return next === -1 ? after : after.slice(0, next);
};

/** The `## Unreleased` body with its bump marker and surrounding whitespace stripped: `''` for
 * an empty section, null for no section. */
export const changelog_unreleased_content = (changelog: string): string | null => {
	const section = changelog_section(changelog, 'Unreleased');
	return section === null ? null : section.replace(UNRELEASED_BUMP_MARKER, '').trim();
};

/** The bump level the `## Unreleased` marker declares, or null. */
export const changelog_declared_bump = (changelog: string): BumpLevel | null => {
	const section = changelog_section(changelog, 'Unreleased');
	if (section === null) return null;
	return (UNRELEASED_BUMP_MARKER.exec(section)?.[1] as BumpLevel | undefined) ?? null;
};

export type ChangelogStampOutcome =
	/** `## <version>` was already present (a resumed release); the text is unchanged. */
	| 'already_stamped'
	/** `## Unreleased` and its marker became `## <version>` under a fresh `## Unreleased`. */
	| 'stamped'
	/** The same, from an `## Unreleased` with no marker. */
	| 'stamped_without_marker'
	/** No `## Unreleased` at all; the text is unchanged. */
	| 'no_unreleased';

export interface ChangelogStamp {
	outcome: ChangelogStampOutcome;
	text: string;
}

/** Stamp `## Unreleased` into `## <version>`, dropping its bump marker, under a fresh empty
 * `## Unreleased` that declares a patch. Idempotent: an already-stamped version is left alone. */
export const changelog_stamp = (changelog: string, version: string): ChangelogStamp => {
	if (heading_re(version).test(changelog)) return {outcome: 'already_stamped', text: changelog};
	if (UNRELEASED_HEADING_WITH_MARKER.test(changelog)) {
		return {
			outcome: 'stamped',
			text: changelog.replace(UNRELEASED_HEADING_WITH_MARKER, `${FRESH_UNRELEASED}## ${version}\n`),
		};
	}
	if (UNRELEASED_HEADING.test(changelog)) {
		return {
			outcome: 'stamped_without_marker',
			text: changelog.replace(UNRELEASED_HEADING, `${FRESH_UNRELEASED}## ${version}`),
		};
	}
	return {outcome: 'no_unreleased', text: changelog};
};
