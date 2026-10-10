-- The accept side of the optional name: a quoted `"on"` is a name, not the clause, so
-- `CREATE INDEX "on" ON …` declares an index called `on`.
-- @setup
CREATE TABLE ciu24 (a INT);
CREATE INDEX "on" ON ciu24 (a);
-- @test
SELECT indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu24';
