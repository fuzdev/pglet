-- The neighbour of the typed-then-`"any"` fixture: the select list is analysed before
-- `WHERE`, so the `"any"` argument is reached while `$1` is still untyped, and the
-- comparison in `WHERE` types it too late.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
-- @expect-error: the "any" argument is reached before the comparison that types the parameter: 42P08
-- @expect-message
SELECT jsonb_build_object('k', $1) AS j FROM up_t WHERE name = $1;
