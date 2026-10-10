-- `CREATE INDEX ON <table> (<column>)` names no index, and 18.6 chooses the name:
-- `<table>_<column>_idx` (`ChooseIndexName`). The key column is not the table's first, so
-- a name built from the table's column order instead of the key's reads `ciu1_a_idx`.
-- @setup
CREATE TABLE ciu1 (a INT, b FLOAT8, c TEXT);
CREATE INDEX ON ciu1 (b);
-- @test
SELECT indexname, indexdef::text AS indexdef FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu1';
