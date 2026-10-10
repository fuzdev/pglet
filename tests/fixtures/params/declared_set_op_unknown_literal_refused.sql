-- A string constant the declared type refuses is `22P02`.
-- @param-declared $1 int4 5
-- @expect-error: invalid input syntax for type integer
-- @test
SELECT $1 UNION ALL SELECT 'x';
