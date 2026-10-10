-- A huge but numeric hour field is a datetime field overflow (22008), not a format
-- error (22007): the field parses as a number, then fails the 24:00:00 range check.
-- @test
-- @expect-error: date/time field value out of range
SELECT '9999999999:00:00'::time;
