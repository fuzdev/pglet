-- The `NOT MATERIALIZED` spelling of the same noise word.
-- @test
WITH c(v) AS NOT MATERIALIZED (SELECT 1) SELECT v FROM c;
