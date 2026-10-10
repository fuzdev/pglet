-- A `DELETE` body beside a data-modifying CTE. 18.6 runs the body against the statement's
-- snapshot, which does not hold the row the CTE inserts (`DELETE 0`); run after the CTE,
-- the body would delete it. Refused before anything runs.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: pglet refuses a writing body beside a data-modifying CTE (18.6 answers)
-- @test
WITH w AS (INSERT INTO s VALUES (9) RETURNING x) DELETE FROM s WHERE x = 9;
