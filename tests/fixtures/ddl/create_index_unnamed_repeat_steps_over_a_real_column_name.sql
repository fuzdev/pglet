-- The digit a repeated key part takes steps around the names the key already holds, not
-- only around its own earlier repeats: the key is `a1, a, a`, so the second `a` cannot be
-- `a1` and is `a2`.
-- @setup
CREATE TABLE ciu6 (a INT, a1 INT);
CREATE INDEX ON ciu6 (a1, a, a);
-- @test
SELECT indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu6';
