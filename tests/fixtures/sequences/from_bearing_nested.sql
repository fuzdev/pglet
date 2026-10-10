-- A sequence function in a FROM-bearing query is not a top-level no-FROM
-- projection, so pglet fails loud. Postgres advances the sequence once per row and
-- accepts it, so this documents the deliberate pglet-stricter divergence — any
-- pglet error passes.
-- @setup
CREATE SEQUENCE seq_from;
CREATE TABLE t_from (n INT);
INSERT INTO t_from VALUES (1), (2), (3);
-- @test
SELECT nextval('seq_from') FROM t_from;
-- @expect-error: sequence function only supported as a top-level projection
