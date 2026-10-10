-- The unaliased output column is `substr`, and an untyped literal resolves to text.
-- @test
SELECT substr('abcdef', 2, 3), substr(NULL, 1) AS n;
