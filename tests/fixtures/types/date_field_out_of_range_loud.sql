-- A date with the ISO shape but an out-of-range field (month 13) fails loud with
-- 22008 (datetime_field_overflow), distinct from an unparseable shape (22007).
-- @test
-- @expect-error: date/time field value out of range
SELECT '2026-13-01'::date;
