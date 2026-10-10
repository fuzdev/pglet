-- @setup

-- @test
-- the column type unifies across all rows: an integer and a decimal literal fold to
-- numeric (the same common-type rule set operations use)
SELECT x FROM (VALUES (1), (2.5), (3)) AS t(x) ORDER BY x;
