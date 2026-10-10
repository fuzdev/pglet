-- A general expression is a column DEFAULT, stored **unevaluated** and evaluated per
-- row — 18.6's own model, which `pg_attrdef` holds as the post-parse-analysis tree.
-- Replaces a fixture that pinned pglet's blanket refusal of this whole shape.
-- @setup
CREATE TABLE def_expr (a INT, b INT DEFAULT (1 + 1));
INSERT INTO def_expr (a) VALUES (1);
-- @test
SELECT a, b FROM def_expr;
