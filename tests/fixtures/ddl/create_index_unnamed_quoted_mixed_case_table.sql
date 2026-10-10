-- The table's name enters the index name as the catalog holds it: a quoted mixed-case
-- table keeps its case, and so does a quoted column with a space in it.
-- @setup
CREATE TABLE "CiuMixed" (a INT, "Two Words" INT);
CREATE INDEX ON "CiuMixed" (a);
CREATE INDEX ON "CiuMixed" ("Two Words", a);
-- @test
SELECT indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'CiuMixed' ORDER BY indexname;
