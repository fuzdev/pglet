-- `'…'::regclass` over a name no relation holds is still `42P01`.
-- @expect-error: relation "rgs_nosuch" does not exist
-- @test
SELECT 'rgs_nosuch'::regclass::text;
