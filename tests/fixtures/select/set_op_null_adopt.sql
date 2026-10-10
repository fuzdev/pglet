-- A bare untyped NULL column adopts the sibling side's concrete type: the output
-- column is integer, not text, so the two distinct rows {NULL, 1} order correctly
-- (integer NULLs sort last under ASC). Postgres types this column `integer` too.
-- @test
SELECT NULL UNION SELECT 1 ORDER BY 1;
