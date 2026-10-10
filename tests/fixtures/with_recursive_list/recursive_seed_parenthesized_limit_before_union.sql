-- A parenthesized non-recursive term carrying its own `LIMIT` merges nothing: the set
-- operation after it keeps its clauses apart.
-- @test
WITH RECURSIVE r(n) AS ((SELECT 1 LIMIT 1) UNION ALL SELECT n + 1 FROM r WHERE n < 3) SELECT * FROM r;
