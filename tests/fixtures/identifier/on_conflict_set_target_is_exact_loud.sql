-- The `DO UPDATE SET` target's exactness, guarded apart from the arbiter's: the arbiter
-- here matches the catalog exactly, so the SET target is what decides — `val` is not the
-- column `"Val"`, `42703` on both engines.
--
-- Two exact lookups stand behind that answer. `lower_on_conflict` resolves the target at
-- plan time and refuses `val` there, as 18.6 does in parse analysis, whether or not the
-- row conflicts (measured with a free key: `42703` on both). `apply_do_update` looks the
-- target up again when a row conflicts, which is why this row conflicts on purpose.
-- Measured: with the plan-time refusal removed, the executor's lookup answers `42703` and
-- this file stays green; with the executor's lookup made case-insensitive, the plan-time
-- refusal answers and it stays green; with both, the upsert succeeds and this file goes
-- red (*expected error but got success*).
-- @profile-lock
-- @setup
CREATE TABLE ocset ("UserId" INT PRIMARY KEY, "Val" TEXT);
INSERT INTO ocset ("UserId", "Val") VALUES (1, 'a');

-- @test
INSERT INTO ocset ("UserId", "Val") VALUES (1, 'b') ON CONFLICT ("UserId") DO UPDATE SET val = 'z';
-- @expect-error: the column is named `Val`, and `val` is a different name (42703)
