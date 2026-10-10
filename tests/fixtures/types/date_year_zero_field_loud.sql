-- A year-0 field is outside the date domain (0001-01-01 through 9999-12-31) and 18.6
-- refuses it too, as a field overflow quoting the input — so both the code and the
-- message are 18.6's. Accepted, it would render `0000-12-31`, which no Postgres input
-- function reads back.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '0000-12-31'::date;
