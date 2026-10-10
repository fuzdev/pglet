-- An unrecognized date_trunc field is 22023 (invalid_parameter_value) in both pglet
-- and Postgres — neither token table knows it.
-- @test
-- @expect-error: unit "not_a_unit" not recognized
SELECT date_trunc('not_a_unit', '2024-03-15 12:34:56'::timestamp);
