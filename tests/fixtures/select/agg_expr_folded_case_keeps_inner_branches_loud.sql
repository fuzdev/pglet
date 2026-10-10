-- A `CASE` a constant condition reduces to one arm keeps that arm whole: the inner
-- `CASE`'s condition is not constant, so its untaken `ELSE` aggregate still runs. An
-- arm evaluated only as far as the walk reads it would answer `1` per group.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN true THEN (CASE WHEN a > 0 THEN 1 ELSE sum(1/b) END) END FROM u GROUP BY a;
