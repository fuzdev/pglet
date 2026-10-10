-- The mirror of `insert/on_conflict_error_outranks_returning.sql`: the `ON CONFLICT`
-- action now carries the unbindable bare key (`42703`) and `RETURNING` the bogus
-- qualifier (`42P01`). 18.6 still reports the **action's** code, which is what identifies
-- the rule as *clause order* rather than a fixed preference between the two SQLSTATEs.
--
-- Neither polarity discriminates alone. An engine that always reported `42P01` would pass
-- the sibling and fail here; one that always reported `42703` would do the reverse; one
-- that visited `RETURNING` first would fail both.
-- @profile-lock
-- @setup
CREATE TABLE rlo_target (id INT PRIMARY KEY, v INT);
CREATE TABLE rlo_src (b INT);
INSERT INTO rlo_target VALUES (1, 1);

-- @expect-error: column "aaa" does not exist (42703) — the action, not RETURNING's 42P01
-- @test
INSERT INTO rlo_target VALUES (4, 4)
  ON CONFLICT (id) DO UPDATE SET v = (SELECT b FROM rlo_src ORDER BY aaa)
  RETURNING (SELECT b FROM rlo_src WHERE zzzrel.q = 1);
