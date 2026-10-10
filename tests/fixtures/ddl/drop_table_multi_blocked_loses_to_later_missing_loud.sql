-- Every phase-1 error outranks every phase-2 one: a dependency refusal earned by the
-- FIRST name loses to a missing name written after it (`42P01`, not `2BP01`). Red the
-- moment dependency resolution happens inside the phase-1 loop.
-- Mirrored by `drop_table_multi_blocked_wins_when_every_name_resolves_loud.sql`, which
-- is `2BP01` over the same setup once every name resolves.
-- @setup
CREATE TABLE dbl (id INT PRIMARY KEY);
CREATE TABLE dblchild (x INT REFERENCES dbl(id));
CREATE TABLE dblother (z INT);

-- @expect-error: table "dbl_nosuch" does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE dbl, dbl_nosuch;
