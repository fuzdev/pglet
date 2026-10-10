-- `IS DISTINCT FROM` is a predicate, not a `FROM` clause: the column after it is no
-- reference to the later CTE of the same name, so there is no cycle.
-- @setup
CREATE TABLE tt (r INT, q INT, d DATE);
INSERT INTO tt VALUES (5, 6, DATE '2020-01-02');
-- @test
WITH RECURSIVE a AS (SELECT q IS DISTINCT FROM r AS dd FROM tt),
  r AS (SELECT * FROM a)
SELECT * FROM r;
