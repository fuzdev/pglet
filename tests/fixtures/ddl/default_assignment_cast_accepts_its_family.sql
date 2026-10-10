-- The **accept** side of the cook's assignment cast, which is the half a new gate gets
-- wrong: every pair `can_coerce_type(… COERCION_ASSIGNMENT)` reaches must still be
-- accepted, and a bare string constant is Postgres's `unknown` — resolved by the target
-- column through its input function, never gated. Fourteen columns, each a pair 18.6
-- accepts: the numeric family in both directions, the temporal family, an I/O coercion
-- into a string-category target (which takes *every* type), and the unknown constants
-- the gate must not see at all.
-- @no-declared-types: one column diverges, `column_default` — 18.6 sends `character
-- varying` (OID 1043), the *base* type its `character_data` domain resolves to in a
-- RowDescription, and pglet's synthesized view declares `text` (25). `column_name`
-- agrees at 25 on both, because this `@test` casts it (measured on both engines).
-- Same catalog-view type gap the sibling
-- `ddl/default_numeric_literal_keeps_its_display_scale.sql` waives, and outside this
-- fixture's subject, which is which defaults the cook accepts.
-- @setup
CREATE TABLE dac (
  a INT DEFAULT 5,
  b INT DEFAULT -1,
  c INT DEFAULT 1.5,
  d INT DEFAULT '5',
  e INT DEFAULT 5::int8,
  f INT DEFAULT 1 + 1,
  g NUMERIC DEFAULT 5,
  h FLOAT8 DEFAULT 5,
  i TEXT DEFAULT 5,
  j TEXT DEFAULT true,
  k TEXT DEFAULT '{}'::jsonb,
  l TIMESTAMP DEFAULT '2024-01-01'::date,
  m TEXT DEFAULT concat('a', 'b'),
  n BOOL DEFAULT 't'
);
-- @test
SELECT column_name::text AS column_name, column_default
FROM information_schema.columns
WHERE table_name::text = 'dac'
ORDER BY ordinal_position ASC
