-- `IF EXISTS` turns the missing-relation case into a NOTICE, and the statement that
-- follows still runs.
-- @setup
CREATE SEQUENCE ifex_seq START WITH 3;
ALTER SEQUENCE IF EXISTS no_such_sequence_at_all RESTART WITH 99;
-- @test
SELECT nextval('ifex_seq');
