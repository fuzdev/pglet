-- A no-FROM comparison of an int literal against a bare string literal coerces the
-- string to int, and trailing whitespace is trimmed (like `'1 '::int`), so
-- `1 <> '1 '` is `f`. Previously the no-FROM path text-compared and returned `t`.
-- @test
SELECT 1 <> '1 ' AS ne;
