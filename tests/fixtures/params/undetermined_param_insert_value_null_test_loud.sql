-- Inside one `VALUES` entry: the `CASE` condition's null test precedes the branch the
-- target column types.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: the null test is reached before the occurrence that types the parameter: 42P08
-- @expect-message
INSERT INTO up_t (id, name) VALUES (9, CASE WHEN $1 IS NULL THEN 'a' ELSE $1 END);
