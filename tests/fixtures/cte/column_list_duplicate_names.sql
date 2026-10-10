-- `gram.y`'s `opt_name_list` is a bare `name_list` with no uniqueness rule, and
-- `analyzeCTE` adds none: a duplicated list name is **accepted**. `count(*)` rather than
-- the two columns because the harness collapses same-named output columns.
-- @test
WITH c(x, x) AS (SELECT 1, 2) SELECT count(*) AS n FROM c;
