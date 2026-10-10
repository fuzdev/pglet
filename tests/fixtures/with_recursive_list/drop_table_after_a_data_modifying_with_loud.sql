-- A `DROP TABLE` after a `WITH` list is a syntax error.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: syntax error at DROP (42601)
-- @test
WITH w AS (INSERT INTO s VALUES (5) RETURNING x) DROP TABLE s;
