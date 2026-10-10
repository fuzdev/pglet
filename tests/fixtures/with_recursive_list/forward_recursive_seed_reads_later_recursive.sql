-- A recursive CTE whose seed reads a recursive CTE declared after it. Parsed in
-- declaration order, the later name fell through to a relation the catalog lacks,
-- whose column the working table then typed as `text`, and the recursive term's
-- `m < 13` was refused `42883 text < integer`.
-- @test
WITH RECURSIVE b(m) AS (SELECT n FROM a UNION ALL SELECT m + 10 FROM b WHERE m < 13),
  a(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM a WHERE n < 3)
SELECT m FROM b ORDER BY m;
