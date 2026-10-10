-- A recognized field that is not defined for the source type is 0A000
-- (feature_not_supported): `hour` is a real unit but EXTRACT(hour FROM date) has no
-- time-of-day, so both pglet and Postgres reject it. (date_part('hour', date) instead
-- casts the date to timestamp and succeeds — see date_part.sql.)
-- @test
-- @expect-error: unit "hour" not supported for type date
SELECT extract(hour from '2024-03-15'::date);
