-- `EXTRACT(… FROM …)` reads a column, not a relation: no reference to the later CTE of
-- the same name, so there is no cycle.
-- @setup
CREATE TABLE tt (r INT, q INT, d DATE);
INSERT INTO tt VALUES (5, 6, DATE '2020-01-02');
-- @test
WITH RECURSIVE a AS (SELECT extract(year FROM d) AS y FROM tt),
  d AS (SELECT * FROM a)
SELECT * FROM d;
