-- SUM over a NUMERIC column is exact (result scale = max input scale); AVG is
-- exact with Postgres's division scale.
-- @setup
CREATE TABLE t (grp INT, amount NUMERIC(12, 2));
INSERT INTO t VALUES (1, 10.50), (1, 20.25), (1, 0.30), (2, 100.00), (2, -50.00);

-- @test
SELECT grp, SUM(amount) AS total, COUNT(*) AS n FROM t GROUP BY grp ORDER BY grp;
