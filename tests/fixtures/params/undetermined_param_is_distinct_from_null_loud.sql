-- `x IS DISTINCT FROM NULL` is a null test to 18.6 — its parser rewrites it — so a bare
-- parameter there is left untyped exactly as under `IS NOT NULL`.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: 18.6 leaves the parameter untyped: 42P18
-- @expect-message
SELECT name FROM up_t WHERE $1 IS DISTINCT FROM NULL;
