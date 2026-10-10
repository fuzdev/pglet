-- A join's two sides are analysed before its condition: the derived table's null test
-- is ahead of the comparison in `ON` that types `$1`.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
-- @expect-error: the null test is reached before the occurrence that types the parameter: 42P08
-- @expect-message
SELECT a.id FROM up_t a JOIN (SELECT id FROM up_t WHERE $1 IS NOT NULL) s ON s.id = a.id AND a.name = $1;
