-- Every aggregate a no-FROM projection holds is computed over the one empty row, an expression over one included.
-- @test
SELECT count(*) AS n, sum(1) AS s, max(2) AS mx, min('a'::text) AS mn, avg(3) AS a, count(1) + 1 AS c;
