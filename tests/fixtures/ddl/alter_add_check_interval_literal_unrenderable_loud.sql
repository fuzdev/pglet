-- The `ALTER TABLE … ADD CONSTRAINT … CHECK` spelling of
-- `check_interval_literal_unrenderable_loud.sql`. The routes are separate call sites
-- (`execute_alter_table_add_check` vs `execute_create_table`'s constraint loop), so
-- one covering the other is a claim about the code and not about the corpus.
-- @expect-error: pglet-only — an interval literal in a stored expression cannot render
-- @setup
CREATE TABLE ck_iv_alter (t TIMESTAMPTZ);
-- @test
ALTER TABLE ck_iv_alter ADD CONSTRAINT ck_iv_alter_recent CHECK (t > now() - interval '1 day');
