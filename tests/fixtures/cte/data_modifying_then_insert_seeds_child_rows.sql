-- The seeding idiom: a data-modifying `WITH` inserts parents and the statement inserts a
-- child row for each, referencing it, with the counts and values bound as parameters.
-- @setup
CREATE TABLE dmc (id SERIAL PRIMARY KEY, data JSONB, kind TEXT, created_by INT);
CREATE TABLE dmg (cell_id INT REFERENCES dmc (id), level TEXT, role TEXT, granted_by INT);
-- @param $1 int4 7
-- @param $2 int8 3
-- @param $3 text teacher
-- @test
WITH c AS (
  INSERT INTO dmc (data, kind, created_by)
  SELECT jsonb_build_object('label', 'work ' || i), 'collection', $1
  FROM generate_series(1, $2::bigint) AS i
  RETURNING id)
INSERT INTO dmg (cell_id, level, role, granted_by)
SELECT id, 'viewer', $3, $1 FROM c
RETURNING cell_id, level, role, granted_by;
