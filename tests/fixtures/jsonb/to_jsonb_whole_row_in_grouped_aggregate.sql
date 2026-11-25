-- Inside an aggregate's argument a whole row is read row by row, below the grouping, so a
-- grouping query takes it there.
-- @setup
CREATE TABLE tjg (id INT, g INT);
INSERT INTO tjg VALUES (1, 1), (2, 1), (3, 2);
-- @test
SELECT g, count(to_jsonb(t)) AS n, sum((to_jsonb(t) ->> 'id')::int) AS ids FROM tjg t GROUP BY g ORDER BY g;
