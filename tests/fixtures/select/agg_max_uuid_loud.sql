-- The `max` half of `agg_min_uuid_loud`.
-- @setup
CREATE TABLE agu2 (u UUID);
INSERT INTO agu2 VALUES ('11111111-1111-1111-1111-111111111111');
-- @test
-- @expect-error: function max(uuid) does not exist
SELECT max(u) FROM agu2;
