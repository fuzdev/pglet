-- A declared `numeric` `$N` in a `CASE` result branch beside an `integer` one, in
-- either branch position and read twice: the construct resolves over `numeric`.
-- @param-declared $1 numeric 1.5
-- @test
SELECT CASE WHEN true THEN $1 ELSE 1 END AS then_branch,
       CASE WHEN false THEN 1 ELSE $1 END AS else_branch;
