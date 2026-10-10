-- The `ALTER TABLE … ADD COLUMN … CHECK` spelling of
-- `check_interval_literal_unrenderable_loud.sql` — the third of the three call sites
-- that reach `validate_check_exprs`, and the one whose CHECK resolves against the
-- post-ADD column set.
-- @expect-error: pglet-only — an interval literal in a stored expression cannot render
-- @setup
CREATE TABLE ck_iv_addcol (n INT);
-- @test
ALTER TABLE ck_iv_addcol ADD COLUMN t TIMESTAMPTZ CHECK (t > now() - interval '1 day');
