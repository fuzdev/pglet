-- The `date - date` fold over rows whose values are all integers: the fold is `double
-- precision` for its other branch, so `avg` answers `double precision`, as 18.6 does,
-- rather than the exact `numeric` an `integer` argument takes.
-- @setup
CREATE TABLE t (i INT, f FLOAT8, d DATE, d2 DATE);
INSERT INTO t VALUES (1, 1.5, '2020-01-02', '2020-01-01'), (NULL, 2.5, NULL, NULL),
  (5, NULL, '2021-03-04', '2021-03-01'), (2, NULL, '2020-01-05', '2020-01-01');
-- @test
SELECT avg(COALESCE(d - d2, f + 0)) FROM t WHERE i IS NOT NULL;
