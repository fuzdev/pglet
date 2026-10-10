-- The duplicate is refused only when something **references** it, and the code is
-- `42702` (ambiguous), not `42703` — which is what pglet answered before the rename
-- reached the namespace.
-- @expect-error: a duplicated list name is ambiguous on reference
-- @expect-message
-- @test
WITH c(x, x) AS (SELECT 1, 2) SELECT x FROM c;
