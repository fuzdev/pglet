-- NUMERIC(2,5): the integer budget is precision-scale = -3, so a value ≥ 10^-3
-- overflows loud (Postgres "numeric field overflow").
-- @setup
CREATE TABLE t (a NUMERIC(2,5));
-- @expect-error: numeric field overflow (value too large for the (2,5) budget)
-- @test
INSERT INTO t VALUES (0.00123);
