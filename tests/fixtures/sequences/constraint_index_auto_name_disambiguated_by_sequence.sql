-- The converse: a sequence occupying a constraint index's auto-name pushes the index
-- to the next candidate, because the index has to clear the same namespace.
-- @profile-lock
-- @setup
CREATE SEQUENCE pkdis_pkey;
CREATE TABLE pkdis (a INT PRIMARY KEY);
-- @test
SELECT indexname FROM pg_indexes WHERE tablename = 'pkdis';
