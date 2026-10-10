-- 18.6's grammar spells the window as a single `Iconst`
-- (`TIMESTAMP '(' Iconst ')' opt_timezone`), so a second number is
-- `42601 syntax error at or near ","`. pglet used to accept and discard it.
-- @expect-error: two modifier numbers on a type that carries one
-- @test
SELECT '2020-01-02 03:04:05.6789'::timestamp(3,1);
