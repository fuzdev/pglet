-- An `EXCEPT ALL` inside a derived table in the recursive term refuses it too.
-- @expect-error: recursive reference within EXCEPT (42P19)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL
  (SELECT n + 1 FROM (SELECT n FROM r EXCEPT ALL SELECT 9) z WHERE n < 3))
SELECT * FROM r;
