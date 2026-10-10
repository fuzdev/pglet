-- The noise word is consumed on a CTE with no column list too — it is parsed between
-- `AS` and the body, so the two are independent.
-- @test
WITH c AS MATERIALIZED (SELECT 1 AS v) SELECT v FROM c;
