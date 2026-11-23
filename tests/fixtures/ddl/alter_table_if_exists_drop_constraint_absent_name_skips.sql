-- The spelling `pg_dump --clean --if-exists` writes for every constraint, over a table
-- that exists and a constraint that does not: the `NOTICE` and skip, and the table is
-- left as it was.
-- @setup
CREATE TABLE atie20 (a INT CHECK (a > 0));

-- @test
ALTER TABLE IF EXISTS ONLY atie20 DROP CONSTRAINT IF EXISTS atie20_pkey;
SELECT conname FROM pg_constraint WHERE conname LIKE 'atie20%' ORDER BY conname;
