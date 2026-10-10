-- A bound `$N` is no certain decider, so every arm's aggregate is computed. Bound
-- `false`, the `ELSE` is the arm 18.6's custom plan keeps too, and its aggregate runs.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @param $1 bool false
-- @test
SELECT CASE WHEN $1 THEN 0 ELSE sum(1/b) END AS c FROM u;
