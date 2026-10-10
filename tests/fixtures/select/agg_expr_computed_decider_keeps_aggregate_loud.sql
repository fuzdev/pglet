-- A decider computed from constants is no certain one, so the arm it would decide keeps
-- its aggregate. The condition does not hold, so the `ELSE` is the arm 18.6's planner
-- keeps too, and its aggregate runs.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN 7 % 2 = 0 THEN 0 ELSE sum(1/b) END FROM u;
