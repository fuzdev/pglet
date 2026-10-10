-- split_part with field position zero is an error (22023, invalid_parameter_value)
-- — the positions are 1-based, so zero is out of range.
-- @test
-- @expect-error: field position must not be zero
SELECT split_part('a,b,c', ',', 0);
