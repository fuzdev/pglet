-- A join's two sides are analysed before its condition: the derived table types `$1`
-- ahead of the null test in `ON`.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
SELECT count(*) AS c FROM up_t a JOIN (SELECT id FROM up_t WHERE name = $1) s ON $1 IS NULL;
