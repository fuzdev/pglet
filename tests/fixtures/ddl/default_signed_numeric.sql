-- A **signed** numeric column `DEFAULT` — `DEFAULT -7` / `DEFAULT +7` / `DEFAULT -1.5`.
--
-- Postgres accepts all three. pglet's `parse_column_default` used to be a token-level
-- classifier that recognized only a bare `Number` token, so any leading sign fell
-- through to a fail-loud `unsupported DEFAULT expression` arm, rejecting ordinary DDL.
-- The signs now come through the general expression grammar like every other operand.
--
-- The sign is folded into the stored `Value` rather than kept as a node. Postgres
-- folds `-` the same way (`doNegate`, `gram.y`) and keeps `+` as an operator node.
-- That asymmetry is observable through `information_schema.columns.column_default`,
-- which on 18.6 renders
-- `DEFAULT +7` as `(+ 7)` and `DEFAULT -7` as `'-7'::integer`, where pglet renders
-- `7` and `-7`. The fold is kept anyway — it keeps a signed numeric literal on the
-- closed `Literal` arm rather than pushing it into the `Expr` catch-all, where a bare
-- `+7` would persist an `Expr::UnaryPlus` wrapper — and the rendering divergence is
-- recorded in `EDGES.md` rather than pinned here (this
-- fixture asserts the stored values, which do agree).
-- @setup
CREATE TABLE dsn (a INT, b INT DEFAULT -7, c INT DEFAULT +7, d NUMERIC DEFAULT -1.5, e NUMERIC DEFAULT +1.5);
INSERT INTO dsn (a) VALUES (5);
-- @test
SELECT a, b, c, d, e FROM dsn;
