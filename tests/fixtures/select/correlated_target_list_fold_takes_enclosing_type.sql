-- The accept side of the fold: `COALESCE` and `CASE` over a scalar subquery projecting
-- an enclosing `INT` column, the inner table carrying a same-named `text` column. 18.6
-- unifies each with the integer branch beside it; read through the inner column the
-- pair was `text` and `integer`, `42804`.
-- @setup
CREATE TABLE ctg_p (a INT, b INT);
CREATE TABLE ctg_q (c INT, b TEXT);
INSERT INTO ctg_p VALUES (1, 10), (2, 20);
INSERT INTO ctg_q VALUES (5, 'zz');
-- @test
SELECT a,
       coalesce((SELECT ctg_p.b FROM ctg_q), 0) AS folded,
       CASE WHEN a = 1 THEN (SELECT ctg_p.b FROM ctg_q) ELSE 0 END AS cased
  FROM ctg_p ORDER BY a;
