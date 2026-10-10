-- A string literal compared against an explicit cast resolves to the cast's type, as it
-- does against a column of that type: `1.5::numeric = '1.50'` is true, not `42883
-- operator does not exist: numeric = text`.
-- @test
SELECT 1.5::numeric = '1.50' AS eq, '2000-01-02' > '2000-01-01'::date AS gt;
