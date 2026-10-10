-- The accepting mirror of `drop_table_multi_wrong_kind_if_exists_loud.sql`: a name no
-- relation holds is skipped (with a NOTICE) and the rest of the list is dropped.
-- @setup
CREATE TABLE dies (a INT);
CREATE TABLE dieskeep (a INT);

-- @test
DROP TABLE IF EXISTS dies, dies_nosuch;
SELECT count(*)::int AS remaining FROM information_schema.tables
WHERE table_schema = 'public'
  AND (table_name = 'dies'
       OR table_name = 'dieskeep');
