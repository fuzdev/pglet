-- A `date - date` branch is an `integer` the coercion pass types, so a `COALESCE` holding
-- it beside a `float8` branch is unified to `double precision`, 18.6's type for the fold:
-- `avg` over it is not the exact `numeric` rule's, and averages every value as `double
-- precision`. Left unresolved, the fold was declared `integer`, a guess `avg` did not
-- read as certain.
-- @setup
CREATE TABLE t (i INT, f FLOAT8, d DATE, d2 DATE);
INSERT INTO t VALUES (1, 1.5, '2020-01-02', '2020-01-01'), (NULL, 2.5, NULL, NULL),
  (5, NULL, '2021-03-04', '2021-03-01'), (2, NULL, '2020-01-05', '2020-01-01');
-- @test
SELECT avg(COALESCE(d - d2, f + 0)) AS all_rows,
  avg(COALESCE(abs(d - d2), f + 0)) AS abs_branch,
  avg(COALESCE(i + (d - d2), f + 0)) AS mixed_branch
FROM t;
