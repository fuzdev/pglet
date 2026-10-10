-- A well-formed time past the 24:00:00 endpoint fails loud with 22008
-- (datetime_field_overflow), distinct from an unparseable shape (22007).
-- @test
-- @expect-error: date/time field value out of range
SELECT '25:00:00'::time;
