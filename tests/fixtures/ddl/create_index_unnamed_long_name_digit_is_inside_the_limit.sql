-- The digit that steps around a taken name goes on the **label**, and the name is cut
-- again around it: the second and third indexes over this key are still 63 bytes, each
-- one byte shorter in the column part than the first. Appending the digit to the finished
-- 63-byte name would give 64 bytes; cutting the finished name back to 63 would lose the
-- digit and never terminate.
-- @setup
CREATE TABLE ciu16_tttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo INT);
CREATE INDEX ON ciu16_tttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo);
CREATE INDEX ON ciu16_tttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo);
CREATE INDEX ON ciu16_tttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo);
-- @test
SELECT indexname, length(indexname::text) AS len FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu16_tttttttttttttttttttttttttttttttttt' ORDER BY indexname;
