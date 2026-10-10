-- A `WITH RECURSIVE` body with no combinator degrades to an ordinary CTE — and its
-- column list must degrade with it. Written per-arm, this route dropped the list that
-- the genuinely-recursive path applies, so `SELECT n FROM r` was `42703`.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1) SELECT n FROM r;
