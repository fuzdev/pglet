-- The control for `default_b_expr_excludes_a_quantified_comparison.sql`:
-- `ALTER COLUMN … SET DEFAULT` is `alter_column_default` over full `a_expr`, so the
-- identical unparenthesized expression is accepted on both engines. One route alone
-- is satisfied by an engine that takes one grammar everywhere.
-- @setup
CREATE TABLE sd_quant (k INT, b BOOL);
ALTER TABLE sd_quant ALTER COLUMN b SET DEFAULT 5 = ANY(ARRAY[1,2]);
INSERT INTO sd_quant (k) VALUES (1);
-- @test
SELECT k, b FROM sd_quant;
