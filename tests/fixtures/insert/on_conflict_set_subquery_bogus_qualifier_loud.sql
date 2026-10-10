-- The `42P01` arm of
-- `insert/on_conflict_set_subquery_key_binds_to_nothing_loud.sql` — a qualifier the
-- action's namespace (the target plus `EXCLUDED`) does not name. `check_binding` reaches
-- it through a different tier than the `42703`, and this spelling wrote `(2, 2)` too.
--
-- No conflict, and `ocq_src` empty, for the reasons its sibling states.
-- @profile-lock
-- @setup
CREATE TABLE ocq_target (id INT PRIMARY KEY, v INT);
CREATE TABLE ocq_src (b INT);
INSERT INTO ocq_target VALUES (1, 1);

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
INSERT INTO ocq_target VALUES (2, 2)
  ON CONFLICT (id) DO UPDATE SET v = (SELECT b FROM ocq_src WHERE zzz.q = 1);
