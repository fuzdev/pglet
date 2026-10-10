-- A searched `CASE`'s bare condition is `boolean` before its result is analysed.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 bool true
-- @test
SELECT CASE WHEN $1 THEN $1 IS NULL END AS b;
