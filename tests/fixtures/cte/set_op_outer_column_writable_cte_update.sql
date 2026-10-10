-- A data-modifying CTE's `WHERE` over a set operation naming the updated `numeric`
-- column: `COALESCE` is `numeric`, so the row `1.25` is updated and `1` is not.
-- @setup
CREATE TABLE sowc (n NUMERIC, k INT);
INSERT INTO sowc VALUES (1.25, 1), (1, 1);
-- @test
WITH d AS (UPDATE sowc SET k = k + 1 WHERE coalesce((SELECT n EXCEPT SELECT 1), 2) = 1.25 RETURNING n, k) SELECT * FROM d;
