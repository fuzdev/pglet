-- A table alias is no reference either: `tt AS r` does not read the later CTE `r`.
-- @setup
CREATE TABLE tt (r INT, q INT, d DATE);
INSERT INTO tt VALUES (5, 6, DATE '2020-01-02');
-- @test
WITH RECURSIVE a AS (SELECT r.q FROM tt AS r),
  r AS (SELECT * FROM a)
SELECT q FROM r;
