-- A `VALUES` list in `FROM` types its bare parameter before `WHERE` is analysed.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
SELECT count(*) AS c FROM (VALUES ($1)) v WHERE $1 IS NULL;
