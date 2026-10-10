-- Control: a parenthesized recursive term is a set operation's operand, not a sub-link.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL (SELECT n + 1 FROM r WHERE n < 3))
SELECT n FROM r ORDER BY n;
