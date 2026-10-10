-- `CASCADE` past the sibling default sheds it and keeps the column and its rows.
-- @setup
CREATE TABLE dccsd_t (id SERIAL, b INT DEFAULT nextval('dccsd_t_id_seq'));
INSERT INTO dccsd_t (b) VALUES (DEFAULT), (DEFAULT);
ALTER TABLE dccsd_t DROP COLUMN id CASCADE;
INSERT INTO dccsd_t (b) VALUES (99);
-- @test
SELECT column_name::text AS column_name, column_default::text AS column_default
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'dccsd_t'
ORDER BY ordinal_position;
