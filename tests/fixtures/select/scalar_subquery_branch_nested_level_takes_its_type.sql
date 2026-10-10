-- The scalar subquery names a column two levels out, which the middle level's own
-- lowering cannot type: `COALESCE` there still takes `numeric`.
-- @setup
CREATE TABLE ssbn (n NUMERIC);
INSERT INTO ssbn VALUES (1.25);
-- @test
SELECT (SELECT coalesce((SELECT o.n), 2)) AS c FROM ssbn o;
