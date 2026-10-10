-- Isolates the `c.data @> jsonb_build_object('kind', $1::text)` clause from an
-- earlier `cell_list` read (fuz_app's `query_cell_list`, `db/cell_queries.ts`,
-- now filters a top-level `kind` column instead): a JSONB containment filter over a
-- `data JSONB` column, with the kind built as a single-key jsonb object from a
-- bound text parameter. Rows cover a matching kind, a non-matching kind, and a
-- row with no `kind` key at all.
-- (The full cell_list query also needs correlated EXISTS, which is out of scope;
-- only the containment clause is lifted here.)
-- @setup
CREATE TABLE cell (
  id    UUID PRIMARY KEY,
  data  JSONB NOT NULL
);
INSERT INTO cell (id, data) VALUES
  ('11111111-1111-1111-1111-111111111111', '{"kind":"doc"}'),
  ('22222222-2222-2222-2222-222222222222', '{"kind":"note"}'),
  ('33333333-3333-3333-3333-333333333333', '{"other":"x"}');
-- @param $1 text doc
-- @test
SELECT id FROM cell WHERE data @> jsonb_build_object('kind', $1::text) ORDER BY id;
