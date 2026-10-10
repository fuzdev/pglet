-- An interval whose field magnitude overflows the representable range at parse time
-- fails loud with 22015 (interval_field_overflow), distinct from a malformed spelling
-- (22007) and from date/time field overflow (22008).
-- @test
-- @expect-error: interval field value out of range
SELECT INTERVAL '999999999999 years';
