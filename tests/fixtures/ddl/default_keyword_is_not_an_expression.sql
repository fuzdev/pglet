-- A bare `DEFAULT` inside a DEFAULT expression is `42601` on 18.6 — the same
-- `DEFAULT is not allowed in this context` sentence a stored CHECK expression takes
-- for `CHECK (a = DEFAULT)`. Reached here through `ALTER COLUMN … SET DEFAULT`,
-- whose grammar is the full `a_expr` (a column constraint's is the restricted
-- `b_expr`, where the same spelling is a plain syntax error instead).
-- @expect-error: DEFAULT is not an expression (42601)
-- @setup
CREATE TABLE dfl (a INT, b INT);
-- @test
ALTER TABLE dfl ALTER COLUMN b SET DEFAULT DEFAULT;
