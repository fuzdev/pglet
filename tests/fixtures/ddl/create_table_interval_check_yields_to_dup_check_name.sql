-- The reverse of `create_table_dup_check_name_beats_interval_check.sql`, and the one
-- direction of that pair that used to diverge: with the interval CHECK written
-- **first**, 18.6 cooks it (it accepts an interval literal anywhere) and reports the
-- duplicate name behind it, `42710`. pglet answered `0A000` while the render probe
-- rode inside the per-constraint cook — this kind lives in the same loop as the
-- naming, so it was the one kind whose divergence source order could hide.
--
-- The pair is what carries the claim. Alone, either fixture is consistent with an
-- engine that decides by which check it reaches first.
-- @expect-error: the duplicate CHECK name behind it, not the interval CHECK (42710)
-- @test
CREATE TABLE iv_dup_rev (
  t TIMESTAMPTZ,
  CHECK (t > now() - interval '1 day'),
  CONSTRAINT iv_dup_rev_d CHECK (t IS NOT NULL),
  CONSTRAINT iv_dup_rev_d CHECK (t IS NOT NULL)
);
