-- ...and `currval`, whose own 55000 "not yet defined" never gets a chance.
-- @setup
CREATE TABLE cvt_host (a INT);
-- @test
SELECT currval('cvt_host');
-- @expect-error: cannot open relation, not a sequence (42809)
