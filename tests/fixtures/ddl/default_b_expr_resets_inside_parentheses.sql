-- A `(` resets `b_expr` to `a_expr` on both engines, so the `NOT` excluded above is
-- ordinary inside parentheses — and the mode is **restored** on the way out, which
-- `default_b_expr_postfix_after_parentheses.sql` pins from the other side.
-- @setup
CREATE TABLE def_paren (k INT, b INT DEFAULT 1 + (NOT true)::int);
INSERT INTO def_paren (k) VALUES (1);
-- @test
SELECT k, b FROM def_paren;
