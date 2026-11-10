import { assert, test } from 'vitest';

import {
	changelog_declared_bump,
	changelog_section,
	changelog_stamp,
	changelog_unreleased_content
} from '#lib/release/changelog.ts';

const changelog = `# pglet changelog

Prose that mentions \`## Unreleased\` inline.

## Unreleased
<!-- bump: minor -->

- feat: pending

## 0.1.1

- fix: a thing

## 0.1.0

- first publish
`;

test('changelog_section returns the body up to the next heading', () => {
	assert.deepEqual(changelog_section(changelog, '0.1.1'), '\n\n- fix: a thing\n\n');
	assert.deepEqual(changelog_section(changelog, '0.1.0'), '\n\n- first publish\n');
	assert.deepEqual(changelog_section(changelog, '0.2.0'), null);
});

test('changelog_section matches a version literally', () => {
	assert.deepEqual(changelog_section('## 0x1x1\n\nbody\n', '0.1.1'), null);
});

test('changelog_unreleased_content strips the marker', () => {
	assert.deepEqual(changelog_unreleased_content(changelog), '- feat: pending');
	assert.deepEqual(
		changelog_unreleased_content('## Unreleased\n<!-- bump: patch -->\n\n## 0.1.0\n'),
		''
	);
	assert.deepEqual(changelog_unreleased_content('## 0.1.0\n'), null);
});

test('changelog_declared_bump reads the marker only on the line after the heading', () => {
	assert.deepEqual(changelog_declared_bump(changelog), 'minor');
	assert.deepEqual(changelog_declared_bump('## Unreleased\n\n<!-- bump: major -->\n- x\n'), null);
	assert.deepEqual(changelog_declared_bump('## Unreleased\n- x\n'), null);
	assert.deepEqual(changelog_declared_bump('## 0.1.0\n'), null);
});

test('a CRLF heading is no heading', () => {
	const crlf = '## Unreleased\r\n<!-- bump: patch -->\r\n- x\r\n';
	assert.deepEqual(changelog_section(crlf, 'Unreleased'), null);
	assert.deepEqual(changelog_stamp(crlf, '0.1.2').outcome, 'no_unreleased');
});

test('changelog_stamp turns Unreleased into the version under a fresh Unreleased', () => {
	const { outcome, text } = changelog_stamp(changelog, '0.2.0');
	assert.deepEqual(outcome, 'stamped');
	assert.deepEqual(changelog_unreleased_content(text), '');
	assert.deepEqual(changelog_declared_bump(text), 'patch');
	assert.deepEqual(changelog_section(text, '0.2.0')?.trim(), '- feat: pending');
	assert.deepEqual(
		text.indexOf('## Unreleased\n<!-- bump: patch -->\n\n## 0.2.0\n\n- feat: pending\n') > 0,
		true
	);
});

test('changelog_stamp is idempotent', () => {
	const once = changelog_stamp(changelog, '0.2.0').text;
	assert.deepEqual(changelog_stamp(once, '0.2.0'), { outcome: 'already_stamped', text: once });
});

test('changelog_stamp without a marker still stamps', () => {
	const { outcome, text } = changelog_stamp('# c\n\n## Unreleased\n\n- x\n', '0.1.2');
	assert.deepEqual(outcome, 'stamped_without_marker');
	assert.deepEqual(text, '# c\n\n## Unreleased\n<!-- bump: patch -->\n\n## 0.1.2\n\n- x\n');
});

test('changelog_stamp leaves a changelog with no Unreleased alone', () => {
	assert.deepEqual(changelog_stamp('## 0.1.0\n', '0.1.1'), {
		outcome: 'no_unreleased',
		text: '## 0.1.0\n'
	});
});
