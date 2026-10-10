-- `ON CONFLICT … DO UPDATE … WHERE 't'` performs the update; the action was
-- silently skipped here.
-- @setup
CREATE TABLE bcoc (k INT PRIMARY KEY, v INT);
INSERT INTO bcoc VALUES (1, 1);
-- @test
INSERT INTO bcoc VALUES (1, 9) ON CONFLICT (k) DO UPDATE SET v = 42 WHERE 't'
  RETURNING k, v;
