-- Two data-modifying CTEs on different tables still order the values of a sequence both
-- tables' defaults take: 18.6 gives `b` the first. Refused.
-- @setup
CREATE SEQUENCE sq;
CREATE TABLE t1 (id INT DEFAULT nextval('sq'), v TEXT);
CREATE TABLE t2 (id INT DEFAULT nextval('sq'), v TEXT);
-- @expect-error: pglet refuses two data-modifying CTEs under RECURSIVE (18.6 answers)
-- @test
WITH RECURSIVE a AS (INSERT INTO t1 (v) VALUES ('a') RETURNING id),
  b AS (INSERT INTO t2 (v) VALUES ('b') RETURNING id)
SELECT 1;
