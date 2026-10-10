-- The `ALTER TABLE … ALTER COLUMN … SET DEFAULT` route gets the same walk as a
-- column constraint's DEFAULT — the two callers differ only in their expression
-- grammar. Positional polarity 1 of 2: the aggregate is first, so `42803`.
-- Mirror: `alter_column_set_default_column_ref.sql` (`0A000`).
-- @expect-error: aggregate before the column reference on the ALTER route (42803)
-- @setup
CREATE TABLE dfm (a INT, b INT);
-- @test
ALTER TABLE dfm ALTER COLUMN b SET DEFAULT count(*) + a;
