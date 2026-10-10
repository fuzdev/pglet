-- A group `HAVING` discards computes none of the select list's aggregates: the `a = 2`
-- group's `sum(1/b)` would divide by its `b = 0`, and the statement answers the `a = 1`
-- group. 18.6 answers the same because its planner moves an aggregate-free `HAVING`
-- clause below the grouping, so the `a = 2` row never reaches the aggregate there —
-- computing the select list's aggregates ahead of `HAVING` would raise `22012` here.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT count(*) AS n, sum(1/b) AS s FROM u GROUP BY a HAVING a = 1;
