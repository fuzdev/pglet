-- A unique build over rows holding `0` and `-0` finds the duplicate 18.6 finds. The
-- build's duplicate scan compares the key encoding, which now has one for a float.
-- @profile-lock
-- @expect-error: the build finds -0 beside 0 (23505)
-- @expect-message
-- @setup
CREATE TABLE fkd (x FLOAT8);
INSERT INTO fkd VALUES ('0.0'::float8), ('-0.0'::float8);
-- @test
CREATE UNIQUE INDEX fkd_x ON fkd (x);
