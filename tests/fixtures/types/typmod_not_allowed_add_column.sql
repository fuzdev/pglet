-- `ADD COLUMN` resolves the declared type through the same resolver, so it takes the
-- same refusal.
-- @setup
CREATE TABLE tmac (a INT);
-- @expect-error: int4 takes no type modifier at ADD COLUMN
-- @expect-message
-- @test
ALTER TABLE tmac ADD COLUMN c int4(7);
