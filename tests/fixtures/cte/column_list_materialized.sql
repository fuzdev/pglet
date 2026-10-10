-- `MATERIALIZED` is an inlining hint pglet has no decision to hint, so it is consumed
-- as a noise word rather than refused.
-- @test
WITH c(v) AS MATERIALIZED (SELECT 1) SELECT v FROM c;
