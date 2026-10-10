-- The array family's equality probe, the shape the spine's own bound `text[]` params
-- would take with `=` instead of `@>`. Same hole, same selector, different type arm:
-- the pre-fix reply was `0A000` (*"data type Text[] has no default operator class
-- for a B-tree index"*) where Postgres answers rows.
--
-- Kept separate from the jsonb equality fixture because the two reach the encoder
-- through different arms.
-- Not `-- @profile-lock` — see `gin_method_eq_param_jsonb.sql`.
-- @setup
CREATE TABLE gin_eq_a (id int4, t text[]);
INSERT INTO gin_eq_a (id, t) VALUES
  (1, '{m}'),
  (2, '{a,b}'),
  (3, '{m}'),
  (4, NULL);
CREATE INDEX gin_eq_a_idx ON gin_eq_a USING gin(t);
-- @param $1 text[] {m}
-- @test
SELECT id, t FROM gin_eq_a WHERE t = $1 ORDER BY id;
