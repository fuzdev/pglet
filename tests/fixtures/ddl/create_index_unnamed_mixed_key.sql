-- A key mixing bare columns and expressions takes each part's own name in key order:
-- the bare column its name, the function its name, the operator `expr`.
-- @setup
CREATE TABLE ciu4 (a INT, t TEXT, z INT);
CREATE INDEX ON ciu4 (z, (a + 1), upper(t), a);
-- @test
SELECT indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu4';
