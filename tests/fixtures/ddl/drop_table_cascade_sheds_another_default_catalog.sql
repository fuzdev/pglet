-- The catalog half of the CASCADE: the surviving column is still there and its
-- `column_default` is gone, which is the fact a row read cannot distinguish from a
-- NULL that was always NULL.
-- @setup
CREATE TABLE dtcadc_owner (a SERIAL, b INT);
CREATE TABLE dtcadc_user (x INT DEFAULT nextval('dtcadc_owner_a_seq'));
DROP TABLE dtcadc_owner CASCADE;
-- @test
SELECT column_name::text AS column_name, column_default::text AS column_default
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'dtcadc_user'
ORDER BY ordinal_position;
