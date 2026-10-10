-- Four arguments match only the zone-name form, which integers do not.
-- @expect-error: function generate_series(integer, integer, integer, integer) does not exist
-- @test
SELECT * FROM generate_series(1, 2, 3, 4);
