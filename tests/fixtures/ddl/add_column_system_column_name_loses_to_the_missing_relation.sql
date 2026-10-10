-- The other side of that position: `AlterTable` opens the relation before any
-- subcommand is prepared, so the `42P01` still outranks the collision — the system
-- column name is early on this route, not first.
-- @expect-error: the missing relation outranks the system column name
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE nosuchtbl_acsl ADD COLUMN ctid INT;
