-- A null test inside the source is ahead of the point the target column types the
-- entry.
-- @profile-lock
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
CREATE TABLE up_kv (k TEXT PRIMARY KEY, v JSONB, n INT);
-- @param $1 text z
-- @param $2 int4 5
-- @test
-- @expect-error: the source's null test is reached before the target column types the entry: 42P08
-- @expect-message
INSERT INTO up_kv (k, n) SELECT $1, $2 FROM up_t WHERE $2 IS NOT NULL;
