-- A `varchar(5)` column carried from a recursive CTE into a later one keeps its
-- declared type.
-- @setup
CREATE TABLE vv (v VARCHAR(5));
INSERT INTO vv VALUES ('ab');
-- @test
WITH RECURSIVE r(v) AS (SELECT v FROM vv UNION ALL SELECT v FROM r WHERE false),
  q AS (SELECT v FROM r)
SELECT v FROM q;
