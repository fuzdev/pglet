-- The `ALTER TABLE … ALTER COLUMN … SET DEFAULT` route, positional polarity 2 of 2
-- — the mirror of `alter_column_set_default_aggregate.sql`. The column reference is
-- reached first, so `0A000`.
-- @expect-error: column reference before the aggregate on the ALTER route (0A000)
-- @setup
CREATE TABLE dfn (a INT, b INT);
-- @test
ALTER TABLE dfn ALTER COLUMN b SET DEFAULT a + count(*);
