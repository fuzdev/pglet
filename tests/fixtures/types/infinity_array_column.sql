-- A `date[]` and a `timestamptz[]` column holding infinities.
-- @setup
CREATE TABLE infarr (d DATE[], ts TIMESTAMPTZ[]);
INSERT INTO infarr VALUES ('{infinity,-infinity,2000-01-01}', ARRAY['-infinity'::timestamptz, 'infinity', NULL]);
-- @test
SELECT d, ts FROM infarr;
