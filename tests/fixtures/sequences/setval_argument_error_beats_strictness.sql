-- A STRICT function still evaluates all its arguments before the call is skipped, so an
-- erroring argument beside a NULL one wins.
-- @setup
CREATE SEQUENCE svn5;
-- @test
SELECT setval('svn5', 1/0, NULL);
-- @expect-error: the argument error outranks the NULL short-circuit (22012)
