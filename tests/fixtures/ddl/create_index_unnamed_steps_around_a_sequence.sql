-- A **sequence** holds the name — `pg_class` keeps tables, indexes and sequences in one
-- pool. The explicit sequence took `ciu13_a_idx` and a `SERIAL` column's own sequence
-- `ciu13_id_seq`, so neither can be an index's name: the index over `a` is `ciu13_a_idx1`.
-- @setup
CREATE SEQUENCE ciu13_a_idx;
CREATE TABLE ciu13 (id SERIAL, a INT);
CREATE INDEX ON ciu13 (a);
-- @test
SELECT indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu13';
