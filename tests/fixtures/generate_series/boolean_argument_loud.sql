-- A `boolean` argument matches no signature.
-- @expect-error: function generate_series(boolean, boolean) does not exist
-- @test
SELECT * FROM generate_series(true, false);
