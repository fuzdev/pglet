-- A `varchar(5)` column carried from one recursive CTE into the seed of a later one.
-- @setup
CREATE TABLE vv (v VARCHAR(5));
INSERT INTO vv VALUES ('ab');
-- @test
WITH RECURSIVE r(v) AS (SELECT v FROM vv UNION ALL SELECT v FROM r WHERE false),
  q(w) AS (SELECT v FROM r UNION ALL SELECT w FROM q WHERE false)
SELECT w FROM q;
