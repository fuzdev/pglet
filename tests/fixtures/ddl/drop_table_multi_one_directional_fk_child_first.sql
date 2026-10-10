-- The same one-directional FK with the child written first — the order in which a
-- per-name loop would happen to succeed. Paired with
-- `drop_table_multi_one_directional_fk_parent_first.sql`, which is the order that
-- would not.
-- @setup
CREATE TABLE dofc (id INT PRIMARY KEY);
CREATE TABLE dofcc (x INT REFERENCES dofc(id));
CREATE TABLE dofckeep (z INT);

-- @test
DROP TABLE dofcc, dofc;
SELECT count(*)::int AS remaining FROM information_schema.tables
WHERE table_schema = 'public'
  AND (table_name = 'dofc'
       OR table_name = 'dofcc'
       OR table_name = 'dofckeep');
