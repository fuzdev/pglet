-- A no-FROM aggregate as a set operation's operand.
-- @test
SELECT count(*) UNION ALL SELECT 2 ORDER BY 1;
