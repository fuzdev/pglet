-- A `VALUES` column's type is the common type of its cells, and an untyped cell — a
-- string constant, a `NULL` — takes it. A column of `smallint`, `integer`, string and
-- `NULL` cells is `integer`, one holding a constant past the `integer` range `bigint`,
-- and one holding a decimal `numeric`; `avg` and `sum` take 18.6's rows for each.
-- @test
SELECT avg(a) AS avg_int, sum(a) AS sum_int, avg(b) AS avg_big, sum(b) AS sum_big,
  avg(c) AS avg_num, sum(c) AS sum_num
FROM (VALUES (1::smallint, 1, 1), (2, 5000000000, 1.5), ('4', '9007199254740993', '2'),
  (NULL, NULL, NULL)) v(a, b, c);
