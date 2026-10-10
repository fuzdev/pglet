-- A body whose first operand is parenthesized and followed by more of the body is not
-- wrapped: its first `(` closes before the `UNION`. The list still resolves in dependency
-- order, so the later `b` is read.
-- @test
WITH RECURSIVE a AS ((SELECT * FROM b) UNION ALL (SELECT 1)), b AS (SELECT 2 AS x)
SELECT * FROM a;
