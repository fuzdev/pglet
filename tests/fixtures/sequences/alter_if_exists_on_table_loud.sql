-- ...but the wrong-kind check runs *before* the `IF EXISTS` skip, so a table target
-- is still 42809 even with `IF EXISTS`. This one does not follow from the two checks
-- individually; it is the order they sit in.
-- @setup
CREATE TABLE ifex_not_a_seq (a INT);
-- @test
ALTER SEQUENCE IF EXISTS ifex_not_a_seq RESTART;
-- @expect-error: cannot open relation, not a sequence (42809)
