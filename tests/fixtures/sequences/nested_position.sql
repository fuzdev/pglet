-- A sequence function in a nested position (here a scalar subquery) is rejected
-- by pglet: sequence functions are supported only as a top-level SELECT
-- projection (and in INSERT ... VALUES). Postgres accepts this, so this documents
-- a deliberate pglet-stricter divergence — any pglet error passes.
-- @setup
CREATE SEQUENCE seq_nested;
-- @test
SELECT (SELECT nextval('seq_nested'));
-- @expect-error: sequence function only supported as a top-level projection
