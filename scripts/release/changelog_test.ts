import {assertEquals} from '@std/assert';

import {
	changelog_declared_bump,
	changelog_section,
	changelog_stamp,
	changelog_unreleased_content,
} from './changelog.ts';

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

Deno.test('changelog_section returns the body up to the next heading', () => {
	assertEquals(changelog_section(changelog, '0.1.1'), '\n\n- fix: a thing\n\n');
	assertEquals(changelog_section(changelog, '0.1.0'), '\n\n- first publish\n');
	assertEquals(changelog_section(changelog, '0.2.0'), null);
});

Deno.test('changelog_section matches a version literally', () => {
	assertEquals(changelog_section('## 0x1x1\n\nbody\n', '0.1.1'), null);
});

Deno.test('changelog_unreleased_content strips the marker', () => {
	assertEquals(changelog_unreleased_content(changelog), '- feat: pending');
	assertEquals(changelog_unreleased_content('## Unreleased\n<!-- bump: patch -->\n\n## 0.1.0\n'), '');
	assertEquals(changelog_unreleased_content('## 0.1.0\n'), null);
});

Deno.test('changelog_declared_bump reads the marker only on the line after the heading', () => {
	assertEquals(changelog_declared_bump(changelog), 'minor');
	assertEquals(changelog_declared_bump('## Unreleased\n\n<!-- bump: major -->\n- x\n'), null);
	assertEquals(changelog_declared_bump('## Unreleased\n- x\n'), null);
	assertEquals(changelog_declared_bump('## 0.1.0\n'), null);
});

Deno.test('a CRLF heading is no heading', () => {
	const crlf = '## Unreleased\r\n<!-- bump: patch -->\r\n- x\r\n';
	assertEquals(changelog_section(crlf, 'Unreleased'), null);
	assertEquals(changelog_stamp(crlf, '0.1.2').outcome, 'no_unreleased');
});

Deno.test('changelog_stamp turns Unreleased into the version under a fresh Unreleased', () => {
	const {outcome, text} = changelog_stamp(changelog, '0.2.0');
	assertEquals(outcome, 'stamped');
	assertEquals(changelog_unreleased_content(text), '');
	assertEquals(changelog_declared_bump(text), 'patch');
	assertEquals(changelog_section(text, '0.2.0')?.trim(), '- feat: pending');
	assertEquals(text.indexOf('## Unreleased\n<!-- bump: patch -->\n\n## 0.2.0\n\n- feat: pending\n') > 0, true);
});

Deno.test('changelog_stamp is idempotent', () => {
	const once = changelog_stamp(changelog, '0.2.0').text;
	assertEquals(changelog_stamp(once, '0.2.0'), {outcome: 'already_stamped', text: once});
});

Deno.test('changelog_stamp without a marker still stamps', () => {
	const {outcome, text} = changelog_stamp('# c\n\n## Unreleased\n\n- x\n', '0.1.2');
	assertEquals(outcome, 'stamped_without_marker');
	assertEquals(text, '# c\n\n## Unreleased\n<!-- bump: patch -->\n\n## 0.1.2\n\n- x\n');
});

Deno.test('changelog_stamp leaves a changelog with no Unreleased alone', () => {
	assertEquals(changelog_stamp('## 0.1.0\n', '0.1.1'), {outcome: 'no_unreleased', text: '## 0.1.0\n'});
});
