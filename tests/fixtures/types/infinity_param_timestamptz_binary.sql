-- A binary `timestamptz` parameter holding the `-infinity` sentinel, stored and returned.
-- @setup
CREATE TABLE infpt (id INT, ts TIMESTAMPTZ);
-- @param $1 timestamptz -infinity
-- @test
INSERT INTO infpt VALUES (1, $1) RETURNING id, ts, isfinite(ts);
