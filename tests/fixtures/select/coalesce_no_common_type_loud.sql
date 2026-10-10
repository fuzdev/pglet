-- Branches in different type *categories* have no common type: Postgres reports
-- `42804` ("COALESCE types integer and text cannot be matched") at parse analysis.
-- pglet used to accept this and return whichever branch was non-NULL, declared as the
-- first branch's type.
-- @setup
CREATE TABLE cn (a INT, t TEXT);
INSERT INTO cn VALUES (1, 'x');
-- @test
-- @expect-error: no common type for integer and text (42804)
SELECT COALESCE(a, t) FROM cn;
