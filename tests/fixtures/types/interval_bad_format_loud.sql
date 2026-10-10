-- A malformed interval literal fails loud with 22007 (invalid_datetime_format),
-- distinct from a parse-time field overflow (22015).
-- @test
-- @expect-error: invalid input syntax for type interval
SELECT INTERVAL 'garbage';
