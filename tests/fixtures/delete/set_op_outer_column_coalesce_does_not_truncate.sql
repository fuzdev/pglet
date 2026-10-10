-- The negative half: no row's `COALESCE` equals `1`, so nothing is deleted. Unified as
-- `integer`, the row `1.25` compared `1 = 1` and was deleted.
-- @setup
CREATE TABLE sotr (n NUMERIC);
INSERT INTO sotr VALUES (1.25), (1);
-- @test
DELETE FROM sotr WHERE coalesce((SELECT n EXCEPT SELECT 1), 2) = 1 RETURNING n;
