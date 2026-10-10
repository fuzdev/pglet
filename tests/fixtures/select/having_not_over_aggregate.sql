-- `HAVING NOT (<aggregate predicate>)` — the boolean-negation half of
-- `group_by_unary_plus_over_aggregate.sql`.
-- @setup
CREATE TABLE hnoa (g INT, x INT);
INSERT INTO hnoa VALUES (1, 10), (1, 20), (2, 5);
-- @test
SELECT g FROM hnoa GROUP BY g HAVING NOT (COUNT(*) > 1) ORDER BY g;
