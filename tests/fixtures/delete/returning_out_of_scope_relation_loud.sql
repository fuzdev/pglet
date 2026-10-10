-- The `RETURNING` namespace is the DML target and nothing else, and the check is
-- **statement-level**: `roo_other` is a real table that this statement never mentions, so
-- naming it is `42P01` whether or not a row matches. The `WHERE` here matches nothing,
-- which is the whole point — the qualifier check used to live only in the executor's
-- per-row projection, so a zero-row statement reached no resolver at all and succeeded,
-- declaring `roo_other.big`'s **`int8`** on the extended leg (the planner looked the
-- qualifier up in the whole catalog) and `int4` on the simple one. Two walkers, two
-- fabricated types, and a query 18.6 refuses outright.
-- @setup
CREATE TABLE roo_target (id INT, b BIGINT);
CREATE TABLE roo_other (id INT, big BIGINT);
INSERT INTO roo_target VALUES (1, 111);
INSERT INTO roo_other VALUES (1, 999);

-- @test
-- @expect-error: missing FROM-clause entry for table "roo_other" (42P01)
DELETE FROM roo_target WHERE id = 999 RETURNING roo_other.big + 0;
