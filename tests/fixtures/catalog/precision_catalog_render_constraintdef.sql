-- **A modifier has a third position, and it is inside the words.** 18.6's
-- `format_type` writes `timestamp(3) without time zone` — the `(p)` after the leading
-- word — where `character varying(3)` and `numeric(4,1)` take it after the whole name.
-- Rendering the second rule at a temporal type emits
-- `timestamp without time zone(3)`, which no 18.6 catalog reader prints.
--
-- Each cast's `p` differs from its column's, so 18.6's `coerce_type` keeps the node
-- rather than eliding it (the no-op-typmod elision pglet does not reproduce is
-- `EDGES.md`'s, and is why the matching spellings are not here).
-- `pg_get_constraintdef` and `check_clause` are two readers of the one Catalog
-- rendering, so both are asserted.
-- @setup
CREATE TABLE tcr (a timestamp(3), b timestamptz(3), c time(3), d interval(3));
ALTER TABLE tcr ADD CONSTRAINT tcr_a_check CHECK (a::timestamp(6) = a);
ALTER TABLE tcr ADD CONSTRAINT tcr_b_check CHECK (b::timestamptz(1) = b);
ALTER TABLE tcr ADD CONSTRAINT tcr_c_check CHECK (c::time(1) = c);
ALTER TABLE tcr ADD CONSTRAINT tcr_d_check CHECK (d::interval(1) = d);
-- @no-declared-types: `check_clause` is `information_schema.character_data` on 18.6
--   (base `varchar`, OID 1043) where pglet declares `text` (25) — measured by removing
--   this waiver, and the only column that diverges. Pre-existing catalog-type
--   modelling, not this fixture's subject.
-- @test
SELECT c.conname, pg_get_constraintdef(c.oid) AS def, k.check_clause
FROM pg_constraint c
JOIN information_schema.check_constraints k ON k.constraint_name = c.conname
WHERE c.conname LIKE 'tcr\_%'
ORDER BY c.conname;
