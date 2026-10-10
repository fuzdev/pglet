-- AVG over NUMERIC is exact, scaled by Postgres's `select_div_scale`.
-- @setup
CREATE TABLE t (amount NUMERIC);
INSERT INTO t VALUES (10), (20), (30);

-- @test
SELECT AVG(amount) AS mean FROM t;
