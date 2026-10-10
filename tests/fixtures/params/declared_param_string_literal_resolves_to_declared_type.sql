-- A string literal compared against a declared `numeric` `$N` resolves to `numeric`,
-- as it does against a column of that type.
-- @param-declared $1 numeric 1.5
-- @test
SELECT $1 = '1.50' AS eq, '1.50' < $1 AS lt;
