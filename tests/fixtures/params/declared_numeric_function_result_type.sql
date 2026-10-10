-- `round` over a declared `numeric` `$N` is `numeric`, as is a `$N` beside a computed
-- `integer` operand the lowering does not type.
-- @param-declared $1 numeric 2.5
-- @test
SELECT round($1) AS r, coalesce($1, length('ab')) AS c;
