-- A unary operator over an aggregate evaluates in a grouping query: unary `+`, which the
-- parser keeps as a node (`Expr::UnaryPlus`), and `-SUM(x)` and `NOT (COUNT(*) > 1)`
-- beside it.
-- @setup
CREATE TABLE gbupa (g INT, x INT);
INSERT INTO gbupa VALUES (1, 10), (1, 20), (2, 5);
-- @test
SELECT g, +SUM(x) AS plus, -SUM(x) AS minus FROM gbupa GROUP BY g ORDER BY g;
