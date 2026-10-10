-- A **window** `ORDER BY` is transformed with `force SQL99 rules` (`parse_clause.c`'s
-- `transformSortClause(… EXPR_KIND_WINDOW_ORDER, true)`), so a bare integer there is
-- never an ordinal — folded or not. Both `2` and `-2` are ordinary constants, every row
-- compares equal, and `row_number()` numbers the input order; `-99` behaves the same
-- rather than raising the `42P10` an out-of-range *top-level* ordinal would.
--
-- This is here because the unary-minus fold made a negative window key an
-- `Expr::Literal` for the first time, and the planner's window path branches on exactly
-- that shape (`answered_by_output`) — measured unchanged on both engines, and pinned so a
-- later widening of the SQL92 ordinal rule into this clause is loud. The trailing
-- top-level `ORDER BY a` is what makes the row order comparable at all.
-- @setup
CREATE TABLE wqn (a INT, b INT);
INSERT INTO wqn VALUES (3, 9), (1, 5), (2, 7);
-- @test
SELECT a,
       b,
       row_number() OVER (ORDER BY 2) AS by_two,
       row_number() OVER (ORDER BY -2) AS by_neg_two,
       row_number() OVER (ORDER BY -99) AS by_neg_ninetynine
FROM wqn
ORDER BY a;
