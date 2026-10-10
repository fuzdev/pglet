-- One argument matches no signature.
-- @expect-error: function generate_series(integer) does not exist
-- @test
SELECT * FROM generate_series(1);
