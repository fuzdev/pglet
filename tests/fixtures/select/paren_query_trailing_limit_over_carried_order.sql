-- A clause the parenthesized query does not already carry is accepted after it: a trailing
-- `LIMIT` over its `ORDER BY`. 18.6 merges the two into one query; pglet merges only its
-- record of which clauses are present and applies the `ORDER BY` first, then the `LIMIT`,
-- which is 18.6's order for this pair (`EDGES.md` row 167 has the pairs where it is not).
-- @setup
CREATE TABLE pqd_s (x INT);
INSERT INTO pqd_s VALUES (2), (1), (3);

-- @test
(SELECT x FROM pqd_s ORDER BY x) LIMIT 2;
