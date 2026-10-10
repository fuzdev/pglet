-- An undeclared `$N` in a set operation is unknown and takes the other side's type,
-- and is inferred as that type.
-- @param $1 int4 5
-- @test
SELECT 1 AS v UNION ALL SELECT $1 ORDER BY 1;
