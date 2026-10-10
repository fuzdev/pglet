-- A duplicate CHECK name written **ahead** of the interval CHECK is `42710` on both
-- engines, because pglet's constraint loop cooks and names one constraint at a time
-- (as `AddRelationNewConstraints` does), so constraint 2's name is decided before
-- constraint 3 is looked at.
--
-- This is the arrangement that agreed even while the render probe rode inside the
-- cook; `create_table_interval_check_yields_to_dup_check_name.sql` is its reverse,
-- which did **not** agree until the probe moved into its own whole-statement pass.
-- Keep the pair: only together do they say that the duplicate-name kind is decided
-- by source order on both engines rather than by which check pglet reaches first.
-- @expect-error: duplicate CHECK name, ahead of the interval CHECK behind it (42710)
-- @test
CREATE TABLE ck_iv_order (
  t TIMESTAMPTZ,
  CONSTRAINT ck_iv_order_dup CHECK (t IS NOT NULL),
  CONSTRAINT ck_iv_order_dup CHECK (t IS NOT NULL),
  CHECK (t > now() - interval '1 day')
);
