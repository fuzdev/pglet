-- A DISTINCT ON expression that is not among the initial ORDER BY expressions
-- fails loud (42P10), matching Postgres, instead of silently picking an arbitrary
-- row per group. Here `dept` (the ON key) never appears in `ORDER BY name`.
-- @expect-error: DISTINCT ON expressions must match initial ORDER BY (42P10)
-- @setup
CREATE TABLE emp (dept TEXT, name TEXT, salary INT);
INSERT INTO emp VALUES ('eng', 'alice', 100), ('sales', 'dan', 90);
-- @test
SELECT DISTINCT ON (dept) dept, name FROM emp ORDER BY name;
