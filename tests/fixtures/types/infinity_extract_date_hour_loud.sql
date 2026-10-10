-- `EXTRACT(hour FROM date)` refuses the field for an infinite date as for a finite one.
-- The sentence names the type differently here, so only the code is compared.
-- @expect-error: unit "hour" not supported for type date
-- @test
SELECT extract(hour FROM date 'infinity');
