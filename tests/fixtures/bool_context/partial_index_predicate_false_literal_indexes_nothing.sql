-- Its mirror: a predicate resolving FALSE leaves every row out of the index, so
-- the same duplicate is written. The pair is what says the two implementations
-- moved together — one alone leaves a row enforced-unique but unindexed, or the
-- reverse.
-- @setup
CREATE TABLE bcpi2 (a INT, b INT);
CREATE UNIQUE INDEX bcpi2_ix ON bcpi2 (a) WHERE 'f';
INSERT INTO bcpi2 VALUES (1, 1);
-- @profile-lock
-- @test
INSERT INTO bcpi2 VALUES (1, 2) RETURNING a, b;
