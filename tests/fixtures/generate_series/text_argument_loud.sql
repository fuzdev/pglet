-- A `text` argument matches no signature.
-- @expect-error: function generate_series(text, unknown) does not exist
-- @test
SELECT * FROM generate_series('a'::text, 'c');
