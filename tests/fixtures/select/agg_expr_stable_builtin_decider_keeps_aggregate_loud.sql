-- `current_user` is stable, so 18.6 does not fold the condition and the `ELSE` aggregate
-- runs. Computing an aggregate only where the `CASE` reads it would answer `0`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN current_user IS NOT NULL THEN 0 ELSE sum(1/b) END FROM u;
