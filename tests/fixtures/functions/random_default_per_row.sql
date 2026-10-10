-- A random() default is volatile: an ADD COLUMN backfill draws once per row, as an
-- INSERT does, so the rows' draws are not one value.
-- @setup
CREATE TABLE rd (id INT, a DOUBLE PRECISION DEFAULT random());
INSERT INTO rd (id) SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6;
ALTER TABLE rd ADD COLUMN b DOUBLE PRECISION DEFAULT random();
-- @test
SELECT count(DISTINCT a) > 1 AS a_varies, count(DISTINCT b) > 1 AS b_varies,
  sum(CASE WHEN a >= 0 AND a < 1 AND b >= 0 AND b < 1 THEN 1 ELSE 0 END) AS in_range
FROM rd;
