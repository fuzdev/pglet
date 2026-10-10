-- ...and because the call is skipped rather than run with a NULL, the counter does not
-- move: the sequence still hands out its first value.
-- @setup
CREATE SEQUENCE svn2;
-- @test
SELECT setval('svn2', NULL); SELECT nextval('svn2');
