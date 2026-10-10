-- The control beside the integer rule: `avg` over `real` and `double precision` is
-- `double precision`, `sum` keeps each float width, and `numeric` and `interval` keep
-- their own type under both aggregates.
-- @setup
CREATE TABLE t (f REAL, d DOUBLE PRECISION, n NUMERIC, iv INTERVAL);
INSERT INTO t VALUES (0.1, 0.1, 3.5, '1 day'), (0.2, 0.2, 4, '1 mon'), (NULL, NULL, NULL, '25 hours');
-- @test
SELECT avg(f) AS af, avg(d) AS ad, avg(n) AS an, avg(iv) AS ai,
  sum(f) AS sf, sum(d) AS sd, sum(n) AS sn, sum(iv) AS si FROM t;
