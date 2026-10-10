-- An unknown-typed (bare string) literal compared against a `jsonb`, `bytea`, or
-- array column: Postgres resolves the literal *to the column's type* through that
-- type's input function, so all three comparisons are legal and match. pglet only
-- inserted the coercing cast for scalar targets, leaving the literal as `text` — so
-- the operand gate saw `jsonb <> text` / `bytea = text` / `text[] = text` and
-- rejected queries Postgres accepts.
-- @setup
CREATE TABLE t (tag TEXT, j JSONB, b BYTEA, ta TEXT[]);
INSERT INTO t VALUES ('x', '{"k": 1}', '\x41', '{a}');
-- @test
SELECT tag, j <> '{"k": 2}' AS jsonb_ne, b = '\x41' AS bytea_eq, ta = '{a}' AS arr_eq
FROM t
WHERE j <> '{"k": 2}' AND b = '\x41' AND ta = '{a}';
