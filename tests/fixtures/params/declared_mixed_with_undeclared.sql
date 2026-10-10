-- One parameter declared and one left for inference: the undeclared `$2` takes the
-- declared `$1`'s `numeric` from the operator and the `COALESCE` it shares with it.
-- @param-declared $1 numeric 1.5
-- @param $2 numeric 1.5
-- @test
SELECT $1 = $2 AS eq, coalesce($2, $1) AS c;
