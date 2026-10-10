-- nullif with an unknown string literal next to an integer: like greatest, the
-- common type resolves to integer and 'a'::int fails to parse (22P02).
-- @test
-- @expect-error: nullif('a', 1) coerces 'a' to int and fails to parse
SELECT nullif('a', 1);
