-- A data-modifying CTE's `SET` list reads the old row too: the swap exchanges each row's
-- two values, and the outer query reads the CTE's `RETURNING` rows.
-- @setup
CREATE TABLE wusw (a INT, b INT);
INSERT INTO wusw VALUES (1, 2), (3, 4);

-- @test
WITH u AS (UPDATE wusw SET a = b, b = a RETURNING *) SELECT a, b FROM u ORDER BY a;
