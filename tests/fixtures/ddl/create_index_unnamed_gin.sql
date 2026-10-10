-- The access method takes no part in the name either: an unnamed `USING gin` index is
-- `<table>_<column>_idx`.
-- @setup
CREATE TABLE ciu10 (a INT, j JSONB, tags TEXT[]);
CREATE INDEX ON ciu10 USING gin (j);
CREATE INDEX ON ciu10 USING gin (tags);
-- @test
SELECT indexname, indexdef::text AS indexdef FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu10' ORDER BY indexname;
