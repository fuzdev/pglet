-- COALESCE over an aggregate whose group is empty: SUM(size) is NULL, so COALESCE
-- falls through to the 0 default — proving the COALESCE branch (not just the bare
-- aggregate) is honored in the aggregate evaluator.
-- @setup
CREATE TABLE fact (hash TEXT, size INT);
-- @test
SELECT COUNT(*)::bigint AS count, COALESCE(SUM(size), 0)::bigint AS total FROM fact;
