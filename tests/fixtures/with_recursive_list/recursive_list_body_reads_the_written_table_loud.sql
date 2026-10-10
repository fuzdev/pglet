-- Under `WITH RECURSIVE`, a body reading the table a data-modifying CTE writes would
-- read the rows the write left, where 18.6 reads the statement's snapshot. Refused.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: pglet refuses a read of a table a data-modifying CTE writes (18.6 answers)
-- @test
WITH RECURSIVE w AS (INSERT INTO s VALUES (9) RETURNING x) SELECT count(*) FROM s;
