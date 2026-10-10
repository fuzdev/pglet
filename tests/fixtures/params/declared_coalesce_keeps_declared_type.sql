-- A `$N` whose type the client declared at `Parse` has that type for parse analysis:
-- `coalesce($1, 1)` with `$1` declared `numeric` resolves over `numeric` and keeps
-- `1.5`, where converting the parameter to the literal's `integer` answers `2`.
-- `EDGES.md` row 146.
-- @param-declared $1 numeric 1.5
-- @test
SELECT coalesce($1, 1) AS c, coalesce(1, $1) AS c_second;
