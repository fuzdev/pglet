-- `avg` carries the same accept-list as `sum` (minus `money`): a timestamp operand
-- is 42883, not a silently-NULL average.
-- @setup
CREATE TABLE aw (at TIMESTAMP);
INSERT INTO aw VALUES ('2026-07-18 12:00:00'), ('2026-07-18 13:00:00');
-- @test
-- @expect-error: no avg(timestamp)
SELECT AVG(at) FROM aw;
