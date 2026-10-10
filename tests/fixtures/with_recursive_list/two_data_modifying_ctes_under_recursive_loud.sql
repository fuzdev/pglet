-- 18.6 runs a data-modifying CTE nothing reads after the statement's query, in the
-- reverse of the resolved order (`b` takes serial 1, `a` serial 2). Two data-modifying
-- CTEs under `WITH RECURSIVE` are refused rather than run in another order.
-- @setup
CREATE TABLE t (id SERIAL, v TEXT);
-- @expect-error: pglet refuses two data-modifying CTEs under RECURSIVE (18.6 answers)
-- @test
WITH RECURSIVE a AS (INSERT INTO t (v) VALUES ('a') RETURNING id),
  b AS (INSERT INTO t (v) VALUES ('b') RETURNING id)
SELECT 1;
