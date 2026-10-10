-- A composite key's column names are joined in **key** order, which here is neither the
-- table's column order nor alphabetical, and a quoted mixed-case column keeps its case in
-- the name. The second statement's key is the first's reversed, so the two names differ.
-- @setup
CREATE TABLE ciu2 (a INT, "B" INT, c INT);
CREATE INDEX ON ciu2 (c, "B", a);
CREATE INDEX ON ciu2 (a, "B", c);
-- @test
SELECT indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu2' ORDER BY indexname;
