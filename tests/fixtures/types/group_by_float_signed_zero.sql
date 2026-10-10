-- GROUP BY over a float8 column: `-0.0` and `0.0` fold into ONE group (Postgres,
-- and values_equal via cmp_f64_total, treat signed zeros equal), and every NaN folds
-- into one group. Regression guard for the signed-zero / NaN key normalization
-- (the raw `{:?}` key split `-0.0` from `0.0`).
-- @setup
CREATE TABLE f (id INT, v FLOAT8);
INSERT INTO f VALUES
  (1, 0.0), (2, -0.0), (3, 0.0),
  (4, 1.5), (5, 1.5),
  (6, 'NaN');
-- @test
SELECT count(*) AS n FROM f GROUP BY v ORDER BY n;
