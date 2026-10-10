-- @setup
-- The two Postgres `ColLabel` positions that admit *any* keyword — even
-- reserved (`select`, `from`) and TYPE_FUNC_NAME (`left`) words a bare column
-- position rejects: the attribute after a `.` (`x.left`), and an `AS` label
-- (`AS select`). The inner derived table labels its columns with keywords via
-- `AS`; the outer query reads them back qualified and re-labels them with more
-- keywords. Postgres accepts the whole thing; pglet must match.

-- @test
SELECT x.left AS select, x.from AS between, x.join AS "values"
FROM (SELECT 10 AS left, 20 AS from, 30 AS join) x;
