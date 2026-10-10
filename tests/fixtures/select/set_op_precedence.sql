-- INTERSECT binds tighter than UNION, so this parses as
-- `1 UNION (2 INTERSECT 2)` = `1 UNION {2}` = {1, 2}. If UNION/INTERSECT were equal
-- and left-associative it would be `(1 UNION 2) INTERSECT 2` = {2}, so the result {1,2}
-- confirms the precedence. `ORDER BY 1` is positional (the first output column).
-- @test
SELECT 1 UNION SELECT 2 INTERSECT SELECT 2 ORDER BY 1;
