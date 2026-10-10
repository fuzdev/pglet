-- The chosen name steps around a taken relation name rather than failing on it
-- (`ChooseRelationName`): a **table** holds `ciu11_a_idx`, so the index is `ciu11_a_idx1`.
-- @setup
CREATE TABLE ciu11 (a INT);
CREATE TABLE ciu11_a_idx (z INT);
CREATE INDEX ON ciu11 (a);
-- @test
SELECT indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu11';
