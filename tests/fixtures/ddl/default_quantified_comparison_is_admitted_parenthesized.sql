-- The `(` **reset** applies to the quantified comparison like every other `a_expr`
-- construct: parenthesized, it is a `c_expr` over full `a_expr` and both engines
-- accept it. Without this row the exclusion above is satisfied by an engine that
-- cannot parse `= ANY` in a `DEFAULT` at all.
-- @setup
CREATE TABLE def_quant_ok (k INT, b BOOL DEFAULT (5 = ANY(ARRAY[1,2])), c BOOL DEFAULT (5 = ANY(ARRAY[5,6])));
INSERT INTO def_quant_ok (k) VALUES (1);
-- @test
SELECT k, b, c FROM def_quant_ok;
