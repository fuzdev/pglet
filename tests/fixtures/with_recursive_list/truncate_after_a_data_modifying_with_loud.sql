-- A `TRUNCATE` after a data-modifying `WITH` list is the same syntax error.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: syntax error at TRUNCATE (42601)
-- @test
WITH w AS (INSERT INTO s VALUES (5) RETURNING x) TRUNCATE s;
