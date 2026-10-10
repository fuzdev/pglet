-- An unrecognized EXTRACT field is 22023 (invalid_parameter_value) in both pglet and
-- Postgres — neither token table (DecodeUnits then DecodeSpecial) knows it.
-- @test
-- @expect-error: unit "nonsense" not recognized
SELECT extract(nonsense from '2024-03-15 13:45:06'::timestamp);
