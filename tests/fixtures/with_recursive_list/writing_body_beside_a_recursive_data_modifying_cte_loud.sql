-- The `WITH RECURSIVE` spelling of a writing body beside a data-modifying CTE.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: pglet refuses a writing body beside a data-modifying CTE (18.6 answers)
-- @test
WITH RECURSIVE w AS (INSERT INTO s VALUES (9) RETURNING x) DELETE FROM s WHERE x = 9;
