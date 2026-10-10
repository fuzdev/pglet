-- A scalar subquery beside a constant is a branch of its own output type: `COALESCE`,
-- `greatest` and `ARRAY[…]` over a `numeric` subquery and an `integer` constant are
-- `numeric`, whether the subquery reads a table or the enclosing row.
-- @setup
CREATE TABLE ssbt (n NUMERIC);
INSERT INTO ssbt VALUES (1.25);
-- @test
SELECT coalesce((SELECT 1.25), 2) AS a,
       coalesce((SELECT n FROM ssbt LIMIT 1), 2) AS b,
       greatest((SELECT n FROM ssbt LIMIT 1), 1) AS c,
       ARRAY[(SELECT o.n), 2] AS d,
       coalesce((SELECT o.n), 2) AS e
FROM ssbt o;
