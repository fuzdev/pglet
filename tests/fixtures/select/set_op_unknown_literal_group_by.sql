-- A `GROUP BY` naming a base column leaves the constant unknown.
-- @setup
CREATE TABLE sgb (a INT);
INSERT INTO sgb VALUES (1), (1);
-- @test
SELECT 1 AS v UNION ALL SELECT '2' FROM sgb GROUP BY a ORDER BY 1;
