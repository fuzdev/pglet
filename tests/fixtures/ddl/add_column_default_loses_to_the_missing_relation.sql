-- On the `ALTER TABLE … ADD COLUMN` route the target relation is resolved before
-- anything, so a missing one is `42P01` on a statement whose default is also bad.
-- @expect-error: the relation is resolved before the default is cooked
-- @test
ALTER TABLE add_no_such_table ADD COLUMN b INT DEFAULT 'x';
