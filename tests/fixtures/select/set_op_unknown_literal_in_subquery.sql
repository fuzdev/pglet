-- A set operation inside `IN (…)` unifies the same way; the subquery's column is
-- `integer`, so the comparison with `a` resolves.
-- @setup
CREATE TABLE sis (a INT);
INSERT INTO sis VALUES (1), (2);
-- @test
SELECT a FROM sis WHERE a IN (SELECT 1 UNION SELECT '2') ORDER BY a;
