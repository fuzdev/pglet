-- No `mod(oid, integer)`: an `oid` has no implicit cast to an integer, so 42883 as the
-- `%` operator's is.
-- @setup
CREATE TABLE mo (id INT PRIMARY KEY);
-- @test
-- @expect-error: function mod(oid, integer) does not exist
-- @no-profile-lock: the refusal reads a catalog view's column, which no index serves
SELECT mod(c.oid, 2) FROM pg_constraint c WHERE c.conname = 'mo_pkey';
