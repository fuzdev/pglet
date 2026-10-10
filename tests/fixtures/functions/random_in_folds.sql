-- random() as a branch of COALESCE, greatest and ARRAY beside an integer: the fold is
-- double precision and the draw is kept, not converted to the integer's type.
-- @setup
CREATE TABLE rf (id INT);
INSERT INTO rf VALUES (1), (2), (3), (4), (5), (6), (7), (8);
-- @test
SELECT count(DISTINCT c) > 4 AS c_varies,
  sum(CASE WHEN c > 0 AND c < 1 THEN 1 ELSE 0 END) AS c_fraction,
  sum(CASE WHEN g > 0 AND g < 1 THEN 1 ELSE 0 END) AS g_fraction,
  sum(CASE WHEN a::text NOT IN ('{0,0}', '{1,0}') THEN 1 ELSE 0 END) AS a_fraction,
  min(c * 0) AS c_type
FROM (SELECT coalesce(random(), 0) AS c, greatest(random(), 0) AS g, ARRAY[random(), 0] AS a FROM rf) s;
