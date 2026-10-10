-- `values` not followed by `(` is a name like any other: the later CTE is resolved first.
-- @test
WITH RECURSIVE q AS (SELECT * FROM values), values AS (SELECT 3 AS x) SELECT * FROM q;
