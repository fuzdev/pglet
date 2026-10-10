-- An `INSERT … SELECT` body beside a data-modifying CTE: 18.6's `SELECT` reads the
-- statement's snapshot (two rows), where running after the CTE would read three. Refused.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
CREATE TABLE u (y INT);
-- @expect-error: pglet refuses a writing body beside a data-modifying CTE (18.6 answers)
-- @test
WITH w AS (INSERT INTO s VALUES (9) RETURNING x) INSERT INTO u SELECT x FROM s;
