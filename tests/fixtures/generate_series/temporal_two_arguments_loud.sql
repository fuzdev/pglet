-- A timestamp series needs its step: two timestamps match no signature.
-- @expect-error: function generate_series(timestamp without time zone, unknown) does not exist
-- @test
SELECT * FROM generate_series('2024-01-01'::timestamp, '2024-01-02');
