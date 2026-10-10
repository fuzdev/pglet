-- The outermost of those three: `ALTER TABLE`'s own relation lookup runs before any
-- command, so a missing relation answers `42P01` and the declared type is never read.
-- @expect-error: the missing relation outranks the refused length
-- @expect-message
-- @test
ALTER TABLE nosuchtbl_rt ADD COLUMN c CHAR(0);
