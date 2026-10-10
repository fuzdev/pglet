-- `column_default` is `pg_get_expr` over the tree **18.6's parse analysis** builds, not
-- over the one pglet stores — so the rules are `get_const_expr` and `get_coercion_expr`
-- (`ruleutils.c`), applied to a tree whose casts to the operand's own type were already
-- folded away by `coerce_type`. Six shapes discriminate, and each was
-- `('…')::type` / a bare signed number here before the catalog rendering.
--
-- Both polarities in one row set: `n` keeps its parens (`(5)::bigint`) because the
-- constant's type is *not* the cast's, `as_` keeps its unlabelled literals because
-- `concat` takes `"any"` and never resolves the unknown, and `b`/`i`/`j` were already
-- bare and must stay so.
-- @setup
CREATE SEQUENCE cds_seq;
CREATE TABLE cds (
  a  TEXT      DEFAULT '5'::text,
  f  UUID      DEFAULT gen_random_uuid()::uuid,
  k  INT       DEFAULT -1,
  l  TEXT      DEFAULT 'a' || 'b',
  m  INT       DEFAULT 5::int4,
  n  INT       DEFAULT 5::int8,
  o  INT8      DEFAULT 5::int4,
  p  NUMERIC   DEFAULT -1.5,
  q  NUMERIC   DEFAULT 1.5::numeric(10,2),
  e  JSONB     DEFAULT '{}'::jsonb,
  x  DATE      DEFAULT '2024-01-01'::date,
  y  BOOLEAN   DEFAULT (NOT true),
  z  INT       DEFAULT - (5),
  w  INT8      DEFAULT 2147483648,
  v  NUMERIC   DEFAULT 1e3,
  u  BYTEA     DEFAULT '\x00'::bytea,
  t  INT       DEFAULT abs(-3),
  s  INT       DEFAULT nextval('cds_seq') + 1,
  b  INT       DEFAULT 5,
  i  BOOLEAN   DEFAULT true,
  j  NUMERIC   DEFAULT 1.5,
  ca TEXT      DEFAULT COALESCE('a', 'b'),
  cb TEXT[]    DEFAULT ARRAY['a', 'b'],
  cc INT[]     DEFAULT ARRAY[1, 2],
  as_ TEXT     DEFAULT concat('a', 'b')
);
-- @test
SELECT column_name::text AS column_name, column_default::text AS column_default
FROM information_schema.columns WHERE table_name::text = 'cds'
ORDER BY ordinal_position ASC
