-- An unparseable date literal fails loud with 22007 (invalid_datetime_format),
-- matching Postgres — distinct from a well-formed but out-of-range field (22008).
-- @test
-- @expect-error: invalid input syntax for type date
SELECT 'garbage'::date;
