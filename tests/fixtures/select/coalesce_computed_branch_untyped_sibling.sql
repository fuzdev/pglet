-- A fold some of whose computed branches pglet cannot type keeps the target it had
-- without the typed ones: beside `f` plus a scalar subquery whose own column is a guess,
-- which the branch reader does not type, `i + 0` does not make the target `integer`, so
-- `3.5` is not cast to `4`. Cast to `text` so the declared types compared are both
-- `text`.
-- @setup
CREATE TABLE t (i INT, f FLOAT8);
INSERT INTO t VALUES (1, 1.5), (NULL, 2.5), (3, NULL);
-- @test
SELECT COALESCE(i + 0, f + (SELECT * FROM unnest(ARRAY[1])))::text AS e FROM t ORDER BY f;
