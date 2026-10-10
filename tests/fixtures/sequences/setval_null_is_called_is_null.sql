-- Strictness is per *argument*: a NULL `is_called` skips the call too, so the value
-- argument beside it is never applied either.
-- @setup
CREATE SEQUENCE svn3;
-- @test
SELECT setval('svn3', 5, NULL); SELECT nextval('svn3');
