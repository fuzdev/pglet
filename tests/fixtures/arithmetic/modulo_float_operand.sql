-- There is no float `%` operator: a float8 (or float4) operand on either side
-- resolves to no operator (Postgres 42883), never a silent float modulo.
-- @test
-- @expect-error: no float modulo operator
SELECT 1.5::float8 % 2;
