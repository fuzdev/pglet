-- Two data-modifying CTEs reading a third run in an order 18.6's executor decides as it
-- pulls rows (`s`, `p`, `q` here). Refused.
-- @setup
CREATE TABLE t (id SERIAL, v TEXT);
-- @expect-error: pglet refuses two data-modifying CTEs under RECURSIVE (18.6 answers)
-- @test
WITH RECURSIVE p AS (INSERT INTO t (v) SELECT 'p' FROM s0 RETURNING id),
  q AS (INSERT INTO t (v) SELECT 'q' FROM s0 RETURNING id),
  s0 AS (INSERT INTO t (v) VALUES ('s') RETURNING id)
SELECT 1;
