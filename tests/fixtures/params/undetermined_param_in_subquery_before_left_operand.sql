-- `x IN (SELECT …)`: the subquery is analysed before the left operand.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
SELECT ($1 IS NULL) IN (SELECT true FROM up_t WHERE name = $1) AS b;
