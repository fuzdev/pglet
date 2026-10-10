-- A partial index's predicate takes no part in the name: the unnamed partial index over
-- `b` is `ciu9_b_idx`, and a second one under a different predicate steps to `idx1`.
-- @setup
CREATE TABLE ciu9 (a INT, b INT);
CREATE INDEX ON ciu9 (b) WHERE a > 0;
CREATE INDEX ON ciu9 (b) WHERE a < 0;
-- @test
SELECT indexname, indexdef::text AS indexdef FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu9' ORDER BY indexname;
