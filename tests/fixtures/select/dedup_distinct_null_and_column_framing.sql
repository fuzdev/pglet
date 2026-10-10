-- @test
-- A NULL is not an empty string, two NULLs are one row, and two columns are not one
-- string run together: `('ab','c')` and `('a','bc')` stay two rows.
SELECT
  (SELECT count(*) FROM (SELECT DISTINCT x FROM (VALUES (NULL::text), (''), (NULL), ('')) v(x)) s) AS null_vs_empty,
  (SELECT count(*) FROM (SELECT DISTINCT x, y FROM (VALUES ('ab', 'c'), ('a', 'bc'), ('ab', 'c')) v(x, y)) s) AS framing,
  (SELECT count(*) FROM (SELECT DISTINCT x, y FROM (VALUES (NULL::text, ''), ('', NULL), (NULL, ''), (NULL, NULL)) v(x, y)) s) AS null_per_column;
