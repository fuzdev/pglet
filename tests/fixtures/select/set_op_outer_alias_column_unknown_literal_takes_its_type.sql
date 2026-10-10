-- The alias-qualified outer column and the constant are one `integer` column, so the
-- `INTERSECT` holds for the row `7` alone.
-- @setup
CREATE TABLE sai (i INT);
INSERT INTO sai VALUES (7), (8);
-- @test
SELECT i FROM sai t WHERE EXISTS (SELECT t.i INTERSECT SELECT '07') ORDER BY i;
