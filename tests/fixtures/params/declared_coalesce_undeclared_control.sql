-- The polarity control for `declared_coalesce_keeps_declared_type`: the same
-- `coalesce($1, 1)` with **no** declared type infers `$1` from its sibling, `integer`,
-- on both engines.
-- @param $1 int4 7
-- @test
SELECT coalesce($1, 1) AS c;
