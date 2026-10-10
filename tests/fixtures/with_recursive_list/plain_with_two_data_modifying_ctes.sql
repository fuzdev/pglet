-- Two data-modifying CTEs of a plain `WITH` on unrelated tables, the body reading only
-- them: accepted as 18.6 accepts it — the refusals of a second writer and of a read of
-- a written table are `WITH RECURSIVE`'s.
-- @setup
CREATE TABLE t1 (v TEXT);
CREATE TABLE t2 (v TEXT);
-- @test
WITH a AS (INSERT INTO t1 VALUES ('a') RETURNING v), b AS (INSERT INTO t2 VALUES ('b') RETURNING v)
SELECT a.v, b.v FROM a, b;
SELECT v FROM t1 UNION ALL SELECT v FROM t2 ORDER BY 1;
