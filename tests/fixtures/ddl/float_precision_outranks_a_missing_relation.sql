-- The other side of the same placement: the relation lookup never runs, because 18.6
-- has already refused the type name in the grammar. `42P01` is what an `ADD COLUMN`
-- over a missing relation answers on both engines when the type is fine.
-- @expect-error: a float precision below 1 bit, ahead of the relation lookup
-- @expect-message
-- @test
ALTER TABLE nosuchtbl ADD COLUMN b float(0);
