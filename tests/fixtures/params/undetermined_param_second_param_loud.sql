-- Two parameters: `$1` is typed in time, `$2` is never typed. The refusal names `$2`.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @param $2 text NULL
-- @test
-- @expect-error: $2 is left untyped: 42P18
-- @expect-message
SELECT count(*) AS c FROM up_t WHERE (name = $1 OR $1 IS NULL) AND $2 IS NULL;
