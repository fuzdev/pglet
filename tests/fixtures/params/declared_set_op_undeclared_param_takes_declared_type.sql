-- An undeclared `$N` in a set operation is unknown, like a string constant, so it takes
-- the declared parameter's type from the other side.
-- @param-declared $1 int4 5
-- @param $2 int4 6
-- @test
SELECT $1 AS v UNION ALL SELECT $2;
