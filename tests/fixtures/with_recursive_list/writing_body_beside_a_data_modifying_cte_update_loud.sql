-- An `UPDATE` body beside a data-modifying CTE: 18.6 updates the two rows its snapshot
-- holds (`UPDATE 2`), where running after the CTE would update three. Refused.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: pglet refuses a writing body beside a data-modifying CTE (18.6 answers)
-- @test
WITH w AS (INSERT INTO s VALUES (9) RETURNING x) UPDATE s SET x = x + 100;
