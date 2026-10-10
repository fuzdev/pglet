-- Column *order* is part of the index specification, so `UNIQUE (a, b)` and
-- `UNIQUE (b, a)` are two different indexes on both engines — the merge compares the
-- key list, not the key set.
-- @setup
CREATE TABLE ctcu (a INT, b INT, UNIQUE (a, b), UNIQUE (b, a));

-- @test
SELECT indexname::text AS indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ctcu' ORDER BY indexname;
