-- Two parameters failing two ways: `$2` is never typed (`42P18`) and `$1` is typed
-- too late (`42P08`). 18.6 raises the second from inside parse analysis, ahead of the
-- sweep that raises the first.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @param $2 text NULL
-- @test
-- @expect-error: 42P08 for $1 ahead of 42P18 for $2
-- @expect-message
SELECT count(*) AS c FROM up_t WHERE $2 IS NULL AND ($1 IS NULL OR name = $1);
