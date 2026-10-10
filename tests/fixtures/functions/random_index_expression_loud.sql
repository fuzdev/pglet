-- random() is VOLATILE: an index expression over it is 42P17.
-- @setup
CREATE TABLE ri (a INT);
-- @test
-- @expect-error: functions in index expression must be marked IMMUTABLE
CREATE INDEX ri_r ON ri ((a + random()));
