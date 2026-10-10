-- GIN-accelerated JSONB containment: `data @> jsonb_build_object(...)` over
-- gin(data). The decomposition is lossy (row 3 has both the key "kind" and the
-- value "note", but not paired), so pglet's recheck Filter must drop the false
-- candidate to match Postgres.
-- @profile-lock
-- @setup
CREATE TABLE gin_data (id int4, data jsonb);
INSERT INTO gin_data (id, data) VALUES
  (1, '{"kind":"note","title":"x"}'),
  (2, '{"kind":"task"}'),
  (3, '{"kind":"x","other":"note"}');
CREATE INDEX gin_data_idx ON gin_data USING gin(data);
-- @test
SELECT id FROM gin_data WHERE data @> jsonb_build_object('kind', 'note') ORDER BY id;
