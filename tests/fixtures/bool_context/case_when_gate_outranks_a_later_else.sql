-- And across arms: the second `WHEN`'s gate fires before the `ELSE` is reached.
-- @setup
CREATE TABLE bcint5 (a INT);
-- @expect-error: argument of CASE/WHEN must be type boolean, not the ELSE arm's 42703
-- @test
SELECT CASE WHEN true THEN 1 WHEN a THEN 2 ELSE nosuchcol END FROM bcint5;
