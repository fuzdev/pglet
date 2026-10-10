-- The same untaken-branch validation applies to a projection over a FROM table (the
-- with-FROM path): the `ELSE 'x'` never runs for the seeded row, but the constant cast
-- is validated at plan time and errors (`22P02`), like Postgres.
-- @setup
CREATE TABLE t (id INT);
INSERT INTO t VALUES (1);
-- @test
-- @expect-error: unparseable untaken ELSE in a FROM-ful projection errors (22P02)
SELECT CASE WHEN 1=1 THEN 1 ELSE 'x' END FROM t;
