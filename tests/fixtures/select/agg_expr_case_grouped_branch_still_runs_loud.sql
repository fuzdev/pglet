-- The untaken branch of a `CASE` decided by a grouped column: every group takes the
-- `THEN`, and the `ELSE` aggregate still runs. Computing an aggregate only where the
-- `CASE` reads it would answer `1` per group.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN a > 0 THEN 1 ELSE sum(1/b) END FROM u GROUP BY a;
