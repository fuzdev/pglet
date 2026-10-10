-- `FLOAT(0)` is below 18.6's window, refused in the grammar (`opt_float`'s
-- `if ($2 < 1)`) rather than by any type's modifier reader.
-- @expect-error: a float precision below 1 bit
-- @expect-message
-- @test
SELECT 1::float(0) AS never_reached;
