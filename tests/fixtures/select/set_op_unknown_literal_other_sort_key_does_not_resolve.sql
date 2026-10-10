-- A sort key that names another expression leaves the constant unknown, and so does a
-- `LIMIT`; both sides below are `integer`.
-- @setup
CREATE TABLE sok (a INT);
INSERT INTO sok VALUES (1);
-- @test
SELECT 1 AS v UNION ALL (SELECT '2' FROM sok ORDER BY a LIMIT 1) ORDER BY 1;
