-- The body reading the written table through a read-only CTE, which inlines where the
-- body reads it. Refused.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: pglet refuses a read of a table a data-modifying CTE writes (18.6 answers)
-- @test
WITH RECURSIVE w AS (INSERT INTO s VALUES (5) RETURNING x), q AS (SELECT * FROM s)
SELECT count(*) FROM q;
