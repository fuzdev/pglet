-- The statement writes `$2` and `$3` and never `$1`. 18.6's sweep names the
-- lowest-numbered slot left without a type, which is the one never written.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @param $2 text NULL
-- @param $3 text NULL
-- @test
-- @expect-error: 42P18 naming $1
-- @expect-message
SELECT $3 IS NULL AS b, $2 AS p;
