-- `transformAExprDistinct` (`parse_expr.c`) short-circuits to
-- `make_nulltest_from_distinct` when either operand is an **undecorated** `NULL`
-- `A_Const`, so 18.6 stores a `NullTest` and never a `DistinctExpr`: the `NOT DISTINCT`
-- polarity reads `IS NULL` and the `DISTINCT` one `IS NOT NULL`, on either operand side.
--
-- Three controls, and each pins a different half of the rule. `d03`/`d04`: a **cast**
-- NULL is a `TypeCast` node, not an `A_Const`, so it keeps the `DistinctExpr` spelling —
-- this is the row that fails if the test is written on the rendered type rather than on
-- the expression's shape. `d09`: an ordinary non-NULL operand keeps the `NOT (…)`
-- wrapping. `d07`/`d08`: the argument is deliberately **not** coerced ("the argument can
-- be any type, so don't coerce it"), so an unknown literal there stays unlabelled.
--
-- The both-bare case is deliberately absent: 18.6 renders `NULL IS NOT DISTINCT FROM
-- NULL` as `(NULL::unknown IS NULL)` and pglet, modelling no `unknown` type name, as
-- `(NULL IS NULL)` — the family's one residual, which a fixture cannot hold.
-- @setup
CREATE TABLE dnl (a INT, s TEXT, v VARCHAR(10));
ALTER TABLE dnl ADD CONSTRAINT d01 CHECK (a IS NOT DISTINCT FROM NULL);
ALTER TABLE dnl ADD CONSTRAINT d02 CHECK (a IS DISTINCT FROM NULL);
ALTER TABLE dnl ADD CONSTRAINT d03 CHECK (a IS NOT DISTINCT FROM NULL::int);
ALTER TABLE dnl ADD CONSTRAINT d04 CHECK (a IS DISTINCT FROM NULL::int);
ALTER TABLE dnl ADD CONSTRAINT d05 CHECK (NULL IS NOT DISTINCT FROM a);
ALTER TABLE dnl ADD CONSTRAINT d06 CHECK (NULL IS DISTINCT FROM a);
ALTER TABLE dnl ADD CONSTRAINT d07 CHECK ('x' IS NOT DISTINCT FROM NULL);
ALTER TABLE dnl ADD CONSTRAINT d08 CHECK (NULL IS NOT DISTINCT FROM 'x');
ALTER TABLE dnl ADD CONSTRAINT d09 CHECK (a IS NOT DISTINCT FROM 1);
ALTER TABLE dnl ADD CONSTRAINT d10 CHECK (s IS NOT DISTINCT FROM NULL);
ALTER TABLE dnl ADD CONSTRAINT d11 CHECK (v IS NOT DISTINCT FROM NULL);
ALTER TABLE dnl ADD CONSTRAINT d12 CHECK ((a + 1) IS NOT DISTINCT FROM NULL);
ALTER TABLE dnl ADD CONSTRAINT d13 CHECK (lower(s) IS DISTINCT FROM NULL);
ALTER TABLE dnl ADD CONSTRAINT d14 CHECK (a IS NOT DISTINCT FROM (NULL));
ALTER TABLE dnl ADD CONSTRAINT d15 CHECK (5 IS NOT DISTINCT FROM NULL);
ALTER TABLE dnl ADD CONSTRAINT d16 CHECK (NULL::text IS NOT DISTINCT FROM NULL);
-- @test
SELECT c.conname::text AS conname, pg_get_constraintdef(c.oid) AS definition
FROM pg_constraint c JOIN pg_class cls ON cls.oid = c.conrelid
WHERE cls.relname::text = 'dnl' AND c.contype::text = 'c'
ORDER BY c.conname::text ASC
