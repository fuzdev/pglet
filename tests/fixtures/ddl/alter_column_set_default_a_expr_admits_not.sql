-- The control for `default_b_expr_excludes_not.sql`: `ALTER TABLE … ALTER COLUMN …
-- SET DEFAULT` is `alter_column_default`, a different production over **full
-- `a_expr`**, so the identical expression is accepted here. The pair is what
-- identifies the grammar rather than the expression — one route alone is satisfied by
-- an engine that takes one grammar everywhere.
-- @setup
CREATE TABLE sd_not (k INT, b BOOL);
ALTER TABLE sd_not ALTER COLUMN b SET DEFAULT NOT true;
INSERT INTO sd_not (k) VALUES (1);
-- @test
SELECT k, b FROM sd_not;
