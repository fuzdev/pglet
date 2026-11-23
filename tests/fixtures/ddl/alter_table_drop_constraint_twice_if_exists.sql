-- Naming one constraint twice under `IF EXISTS` drops it once and skips the second.
-- @setup
CREATE TABLE dcti (a INT CHECK (a > 0), b INT CHECK (b > 0));

-- @test
ALTER TABLE dcti DROP CONSTRAINT IF EXISTS dcti_a_check, DROP CONSTRAINT IF EXISTS dcti_a_check;
SELECT conname FROM pg_constraint WHERE conname LIKE 'dcti%' ORDER BY conname;
