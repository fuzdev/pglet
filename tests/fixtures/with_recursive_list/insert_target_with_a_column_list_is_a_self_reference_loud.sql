-- An `INSERT INTO s (x)` target is a range variable even with a column list after it, so
-- a CTE named `s` that inserts into `s` is a recursive query holding a data-modifying
-- statement.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: recursive query must not contain data-modifying statements (42P19)
-- @test
WITH RECURSIVE s AS (INSERT INTO s (x) VALUES (1) RETURNING x) SELECT * FROM s;
