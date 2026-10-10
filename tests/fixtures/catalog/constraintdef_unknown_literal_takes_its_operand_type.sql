-- A string literal is `unknown` until operator resolution types it, and 18.6 stores the
-- typed `Const` — so a CHECK against a `text` column reads back `'x'::text`, and one
-- against a `date` column would read `'…'::date`. pglet stores the literal untyped and
-- printed it bare, which a schema differ reads as a changed constraint.
--
-- The pattern operators are the same rule plus a second one: `LIKE` is grammar, not an
-- operator name, so the stored node is `~~` / `!~~` / `~~*` and that is what
-- `pg_get_constraintdef` prints.
--
-- Polarities that must not move: `p07` (both operands typed, no unknown to resolve),
-- `p08` (`concat` takes `"any"`, so its arguments stay unknown), `p09`'s `IS NOT NULL`,
-- which has no second operand at all, and `p23`'s `integer` argument position.
-- @setup
CREATE TABLE unk (a INT, s TEXT, n NUMERIC, bo BOOL, ar TEXT[]);
ALTER TABLE unk ADD CONSTRAINT p01 CHECK (s = 'x');
ALTER TABLE unk ADD CONSTRAINT p02 CHECK (s LIKE 'x%');
ALTER TABLE unk ADD CONSTRAINT p03 CHECK (s NOT LIKE 'x%');
ALTER TABLE unk ADD CONSTRAINT p04 CHECK (s ILIKE 'x%');
ALTER TABLE unk ADD CONSTRAINT p05 CHECK (s IS DISTINCT FROM 'q');
ALTER TABLE unk ADD CONSTRAINT p06 CHECK (a <> NULL);
ALTER TABLE unk ADD CONSTRAINT p07 CHECK (a = 1 AND bo = true);
ALTER TABLE unk ADD CONSTRAINT p08 CHECK (concat('a', 'b') <> s);
ALTER TABLE unk ADD CONSTRAINT p09 CHECK (s IS NOT NULL);
ALTER TABLE unk ADD CONSTRAINT p10 CHECK (lower(s) = 'x');
ALTER TABLE unk ADD CONSTRAINT p11 CHECK ((s || 'z') = 'xz');
ALTER TABLE unk ADD CONSTRAINT p12 CHECK (ar @> ARRAY['x']);
ALTER TABLE unk ADD CONSTRAINT p13 CHECK (COALESCE(s, 'd') = 'x');
ALTER TABLE unk ADD CONSTRAINT p14 CHECK (a > -1);
ALTER TABLE unk ADD CONSTRAINT p15 CHECK (n > -1.5);
ALTER TABLE unk ADD CONSTRAINT p16 CHECK (s::text = 'x');
ALTER TABLE unk ADD CONSTRAINT p17 CHECK (a::text = 'x');
ALTER TABLE unk ADD CONSTRAINT p18 CHECK (upper(s) > 'a');
-- A *function argument* position resolves its own unknown, from `pg_proc`'s signature
-- rather than from a sibling operand: `p19`-`p22` carry the literal inside the call.
-- `p23` is the declining polarity in the same call — `lpad`'s middle argument is an
-- `integer` in every overload, so it is not a position this resolves.
ALTER TABLE unk ADD CONSTRAINT p19 CHECK (upper('x') <> s);
ALTER TABLE unk ADD CONSTRAINT p20 CHECK (length('abc') = a);
ALTER TABLE unk ADD CONSTRAINT p21 CHECK (lower('AB') <> s);
ALTER TABLE unk ADD CONSTRAINT p22 CHECK (btrim('  x  ') <> s);
ALTER TABLE unk ADD CONSTRAINT p23 CHECK (lpad('x', 3) <> s);
-- @test
SELECT c.conname::text AS conname, pg_get_constraintdef(c.oid) AS definition
FROM pg_constraint c JOIN pg_class cls ON cls.oid = c.conrelid
WHERE cls.relname::text = 'unk' AND c.contype::text = 'c'
ORDER BY c.conname::text ASC
