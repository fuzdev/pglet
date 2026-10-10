-- The same ordering at the relation: `ALTER TABLE`'s own lookup raises `42P01` before
-- any command runs, so a missing table outranks the modifier refusal.
-- @setup
CREATE TABLE tmord2 (a INT);
-- @expect-error: the relation does not exist, which outranks the type modifier
-- @expect-message
-- @test
ALTER TABLE tmnosuch ADD COLUMN b int4(3);
