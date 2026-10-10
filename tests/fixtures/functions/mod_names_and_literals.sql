-- The unaliased column is `mod`; a string literal beside an integer literal resolves to
-- integer, beside a numeric to numeric; a bare NULL beside a number is typed by it.
-- @test
SELECT mod(7, 3), mod(7, '3') AS iu, mod('7.5', 2.0) AS un, mod(NULL, 3) AS nl;
