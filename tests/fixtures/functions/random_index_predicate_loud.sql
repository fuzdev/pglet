-- random() in a partial index's predicate is 42P17 too.
-- @setup
CREATE TABLE rp (a INT);
-- @test
-- @expect-error: functions in index predicate must be marked IMMUTABLE
CREATE INDEX rp_r ON rp (a) WHERE random() < 0.5;
