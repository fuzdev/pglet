-- The cut never splits a character (`pg_mbcliplen`): both names here are two-byte
-- characters behind an odd-length ASCII prefix, so a byte budget lands inside a character
-- and the name gives that character up whole, leaving a result shorter than 63 bytes.
-- @setup
CREATE TABLE "ciu17_éééééééééééééééééé" ("nüüüüüüüüüüüüüüüüüüüü" INT, a INT);
CREATE INDEX ON "ciu17_éééééééééééééééééé" ("nüüüüüüüüüüüüüüüüüüüü");
CREATE INDEX ON "ciu17_éééééééééééééééééé" ("nüüüüüüüüüüüüüüüüüüüü");
CREATE INDEX ON "ciu17_éééééééééééééééééé" (a, "nüüüüüüüüüüüüüüüüüüüü");
-- @test
SELECT indexname, length(indexname::text) AS chars FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu17_éééééééééééééééééé' ORDER BY indexname;
