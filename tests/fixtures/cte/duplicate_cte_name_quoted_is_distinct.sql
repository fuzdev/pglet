-- The control that identifies the name rather than the gate: an unquoted CTE name folds
-- where the lexer reads it, so `"C"` is a **different** name from `c` and both stand.
-- @test
WITH c(x) AS (SELECT 1), "C"(y) AS (SELECT 2) SELECT x FROM c;
