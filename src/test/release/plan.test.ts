import { assert, test } from 'vitest';

import { release_plan, type ReleasePlan } from '#lib/release/plan.ts';

const pending = '# c\n\n## Unreleased\n<!-- bump: patch -->\n\n- fix: x\n\n## 0.1.1\n\n- y\n';

const problems = (plan: ReleasePlan): Array<string> => (plan.kind === 'fresh' ? plan.problems : []);

test('a matching bump plans a fresh release', () => {
	assert.deepEqual(
		release_plan({ version: '0.1.1', bump: 'patch', sentinel: null, changelog: pending }),
		{
			kind: 'fresh',
			version: '0.1.1',
			level: 'patch',
			target: '0.1.2',
			problems: [],
			stale_sentinel: null
		}
	);
});

test('a sentinel naming the checkout version resumes, ignoring --bump', () => {
	assert.deepEqual(
		release_plan({ version: '0.1.2', bump: 'minor', sentinel: '0.1.2', changelog: null }),
		{
			kind: 'resume',
			version: '0.1.2',
			notes: ['--bump minor is ignored — resuming the interrupted v0.1.2 release']
		}
	);
	assert.deepEqual(
		release_plan({ version: '0.1.2', bump: null, sentinel: '0.1.2', changelog: null }).kind,
		'resume'
	);
});

test('a sentinel naming another version is stale', () => {
	const plan = release_plan({
		version: '0.1.1',
		bump: 'patch',
		sentinel: '0.1.0',
		changelog: pending
	});
	assert.deepEqual(plan.kind === 'fresh' && plan.stale_sentinel, '0.1.0');
	assert.deepEqual(problems(plan), []);
});

test('a fresh release states its level twice', () => {
	const missing = release_plan({
		version: '0.1.1',
		bump: null,
		sentinel: null,
		changelog: pending
	});
	assert.deepEqual(missing.kind === 'fresh' && missing.target, null);
	assert.deepEqual(problems(missing).length, 1);

	const disagrees = release_plan({
		version: '0.1.1',
		bump: 'minor',
		sentinel: null,
		changelog: pending
	});
	assert.deepEqual(disagrees.kind === 'fresh' && disagrees.target, null);
	assert.deepEqual(problems(disagrees).length, 1);

	const unmarked = release_plan({
		version: '0.1.1',
		bump: 'minor',
		sentinel: null,
		changelog: '## Unreleased\n\n- x\n'
	});
	assert.deepEqual(unmarked.kind === 'fresh' && unmarked.target, '0.2.0');
	assert.deepEqual(problems(unmarked).length, 1);
});

test('a fresh release needs notes', () => {
	for (const changelog of [
		null,
		'# c\n\n## 0.1.1\n',
		'## Unreleased\n<!-- bump: patch -->\n\n## 0.1.1\n'
	]) {
		assert.deepEqual(
			problems(release_plan({ version: '0.1.1', bump: 'patch', sentinel: null, changelog })).length,
			1
		);
	}
});

test('a version with no bump is a problem', () => {
	const plan = release_plan({
		version: '0.2.0-alpha.1',
		bump: 'patch',
		sentinel: null,
		changelog: pending
	});
	assert.deepEqual(plan.kind === 'fresh' && plan.target, null);
	assert.deepEqual(problems(plan).length, 1);
});
