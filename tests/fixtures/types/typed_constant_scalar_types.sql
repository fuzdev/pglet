-- `gram.y`'s `AexprConst: ConstTypename Sconst | func_name Sconst` — a typed
-- constant is the string literal read as that type. pglet lowers it to the cast node
-- `'…'::type` lowers to, so the value, the declared type OID and the output column
-- name (the type's canonical spelling, unaliased) all follow from the cast path
-- rather than from a second implementation. The **type name** is read at a separate
-- production, which defaults a parenless fixed-length type differently — no spelling
-- here is one of those; `typed_constant_bare_char_carries_no_length.sql` is.
-- @test
SELECT
  DATE '2024-06-15' AS d,
  TIMESTAMP '2024-06-15 10:00:00' AS ts,
  TIME '10:00:00' AS t,
  NUMERIC '1.5' AS n,
  BOOLEAN 'true' AS b,
  INT4 '7' AS i4,
  TEXT 'x' AS s,
  integer '7' AS i,
  bigint '8' AS i8,
  float8 '1.5' AS f,
  uuid '00000000-0000-0000-0000-000000000001' AS u,
  jsonb '{"a":1}' AS j,
  varchar '5' AS v;
