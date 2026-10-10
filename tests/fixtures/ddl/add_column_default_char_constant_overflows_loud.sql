-- `ADD COLUMN … DEFAULT` **coerces the default expression to the new column's type**,
-- so a constant wider than the column is refused right there. Under the truncation a
-- parenless `char` constant silently took `char(1)`, so the default was `a`, which fits
-- a `VARCHAR(1)` — and the `ALTER` silently succeeded with a truncated default where
-- 18.6 refuses. It is the coercion and **not** a back-fill: an `acd` with zero rows
-- answers the same `22001` on both engines (measured), so no row is seeded here. Both
-- engines answer `22001`; the messages name different types (`EDGES.md` row 95) and
-- `@expect-error` does not compare them.
-- @setup
CREATE TABLE acd (z INT);
-- @test
-- @expect-error: value too long for type character varying(1)
ALTER TABLE acd ADD COLUMN c VARCHAR(1) DEFAULT char 'ab';
