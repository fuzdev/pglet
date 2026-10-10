-- The accept control for the `ON CONFLICT DO UPDATE` half of the `INSERT` clause re-walk:
-- a subquery in a `SET` right-hand side that legitimately names `EXCLUDED` still resolves.
-- Its refusal twins are `insert/on_conflict_set_subquery_key_binds_to_nothing_loud.sql`
-- and its bogus-qualifier sibling; a fix that closed those by giving the nested level the
-- target alone — dropping the `EXCLUDED` pseudo-relation the action's namespace carries —
-- reddens here and nowhere else.
--
-- **No conflict**, so the action is never evaluated: the plan-time gate is the only thing
-- this statement exercises, which is what the control is for. pglet cannot evaluate a
-- subquery inside an executed action (`0A000`, a pre-existing gap), so the conflicting
-- spelling is not writable as a fixture.
-- @setup
CREATE TABLE ocx_target (id INT PRIMARY KEY, v INT);
CREATE TABLE ocx_src (b INT);
INSERT INTO ocx_target VALUES (1, 1);
INSERT INTO ocx_src VALUES (5);

-- @test
INSERT INTO ocx_target VALUES (2, 5)
  ON CONFLICT (id) DO UPDATE SET v = (SELECT count(*) FROM ocx_src WHERE ocx_src.b = excluded.v)
  RETURNING id, v;
