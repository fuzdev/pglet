-- `ALTER TABLE t* DROP COLUMN` and `ADD COLUMN`, the second with a space before the
-- `*` and under `IF EXISTS`, act on the table itself.
-- @setup
CREATE TABLE atsc (a INT, b INT);

-- @test
ALTER TABLE atsc* DROP COLUMN b;
ALTER TABLE IF EXISTS atsc * ADD COLUMN c INT;
SELECT column_name::text AS column_name FROM information_schema.columns WHERE table_name = 'atsc'
ORDER BY column_name;
