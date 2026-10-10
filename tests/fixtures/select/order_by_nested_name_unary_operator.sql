-- The same rule reached through a *prefix* operator rather than an infix one, because
-- the shape test is "is the key a one-field `ColumnRef`" and `-q` is an `OpExpr` just
-- as `q+0` is. Postgres folds unary minus into a constant only for a literal, never
-- over a column reference, so this key resolves against the range table.
--
-- `b AS a` shadows the base `a` and the two run opposite: 18.6 answers `10 20`
-- (descending base `a`, so the row with `a = 2` first) and pglet answered `20 10`.
-- @setup
CREATE TABLE onu (a INT, b INT);
INSERT INTO onu VALUES (1, 20), (2, 10);

-- @test
SELECT b AS a FROM onu ORDER BY -a;
