-- Both the key and the predicate convert to the enum: 18.6 checks the predicate
-- (`CheckPredicate`) before the key expressions (`ComputeIndexAttrs`), so the message
-- names the predicate.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ib (id INT, s TEXT);

-- @profile-lock
-- @test
-- @expect-error: the predicate is checked first
-- @expect-message
CREATE INDEX ib_ix ON ib ((s::mood)) WHERE s::mood = 'ok';
