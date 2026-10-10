-- pglet-stricter, and loud: the role-taking overloads are refused `0A000` — pglet has no
-- role catalog to resolve a role name against — where 18.6 answers (`t` for its
-- `postgres` role). Documentary: 18.6 accepts, so any pglet error passes.
-- @test
-- @expect-error: the role overload is not supported here
SELECT has_schema_privilege('postgres', 'public', 'USAGE');
