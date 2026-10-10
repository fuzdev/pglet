-- A string `DEFAULT` is an *unknown* literal that `coerce_to_target_type` types to the
-- **column's** type, and `get_const_expr` then prints it through that type's **output**
-- function — so `bool DEFAULT 't'` reads `true` and `int DEFAULT '007'` reads `7`, with
-- no label at all. `information_schema`'s literal path carried its own copy of the
-- const rules, and that copy labelled without folding (`'t'::boolean`, `'007'::integer`)
-- while the deparser's copy folded correctly at every other position. One rule, one
-- implementation: every arm now calls `deparse`'s.
--
-- Which types fold and which label is the whole content of the rule, so both sides are
-- here: `int4` bare when non-negative, `'-3'::integer` when not; `int2` / `int8` folded
-- but always labelled, that being `get_const_expr`'s `default:` arm; `numeric` bare only
-- when its output text looks like a float constant (`1.50`, not `'1000'::numeric`);
-- every other type labelled with the literal verbatim.
--
-- `f3`/`f4` are the other half of the same statement's repair: a bare integer `DEFAULT`
-- on a float column is an `int4` `Const` widened by a hidden top-level implicit cast on
-- 18.6, and was `42846 cannot cast type Some(Int8) to Float4` here — a refusal *and* a
-- Rust `Debug` rendering in a wire message.
-- @setup
CREATE TABLE sd (
  b1 BOOLEAN DEFAULT 't', b2 BOOLEAN DEFAULT 'true', b3 BOOLEAN DEFAULT 'yes',
  b4 BOOLEAN DEFAULT '1', b5 BOOLEAN DEFAULT ' f ', b6 BOOLEAN DEFAULT 'off',
  i1 INT DEFAULT '007', i2 INT DEFAULT ' 5 ', i3 INT DEFAULT '-3', i4 INT DEFAULT '0',
  j1 BIGINT DEFAULT '007', j2 BIGINT DEFAULT '3000000000', j3 BIGINT DEFAULT '-7',
  k1 SMALLINT DEFAULT '007', k2 SMALLINT DEFAULT '-7',
  n1 NUMERIC DEFAULT '1.50', n2 NUMERIC(10,2) DEFAULT '1.50', n3 NUMERIC DEFAULT '1e3',
  n4 NUMERIC DEFAULT '+1.5', n5 NUMERIC DEFAULT '-1.5', n6 NUMERIC DEFAULT '0',
  f1 FLOAT8 DEFAULT '1.5', f2 REAL DEFAULT '1.5', f3 FLOAT8 DEFAULT 5, f4 REAL DEFAULT 5,
  t1 TEXT DEFAULT 'hi', v1 VARCHAR(10) DEFAULT 'hi',
  u1 UUID DEFAULT '00000000-0000-0000-0000-000000000000',
  y1 BYTEA DEFAULT '\x01', a1 TEXT[] DEFAULT '{a,b}', d1 DATE DEFAULT '2024-01-01'
);
-- @test
SELECT column_name::text AS column_name, column_default::text AS column_default
FROM information_schema.columns WHERE table_name::text = 'sd'
ORDER BY ordinal_position ASC
