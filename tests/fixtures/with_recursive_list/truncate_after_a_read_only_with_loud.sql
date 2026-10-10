-- A `TRUNCATE` after a read-only `WITH` list is the same syntax error.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: syntax error at TRUNCATE (42601)
-- @test
WITH q AS (SELECT 1) TRUNCATE s;
