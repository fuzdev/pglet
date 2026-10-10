-- A `FETCH` count is a `c_expr` or a signed constant (`select_fetch_first_value`), so
-- `1 + 1` there is a syntax error the raw parse reports ahead of the `FETCH` refusal.
-- @expect-error: syntax error at or near "+"
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 FETCH FIRST 1 + 1 ROWS ONLY)
SELECT * FROM r;
