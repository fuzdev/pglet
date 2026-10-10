-- A projected **expression** over a FROM item carrying an alias must declare the
-- type its own operands give it. The planner's declared-type walk resolved a
-- `Column` leaf through the catalog by name, and a FROM alias is not a catalog
-- table — so `schema.get_table("m")` was `None`, the leaf typed as unknown, and the
-- arithmetic node handed back whatever it had: `text` when neither operand resolved,
-- the *other* operand's type when one did. Measured on 18.6 against this table:
-- `m.s + m.i` declared `text` (OID 25) over four `int4` bytes and `m.b + 0` declared
-- `int4` over eight `int8` bytes — a binary client's decode failure, not a
-- mislabelled-but-readable column.
--
-- Note `alias_unqualified`: the columns are **not** qualified at all. Writing the
-- alias on the FROM item was enough, because the walk fed the scan's *effective*
-- name (the alias when one is written) as its fallback catalog key. That is what
-- makes this ordinary SQL rather than an exotic corner, and it is why the
-- `qualified_*_declared_types` family next door — which qualifies with the **real
-- table name**, so the catalog lookup on the qualifier succeeds — stayed green
-- throughout.
--
-- Discrimination. This file covers the base-`Scan` shape only, and it goes **green**
-- under a fix that merely makes the fallback key the scan's real table name
-- (measured); the `alias_{derived_table,cte,join}_declared_types` siblings are the
-- ones that stay red there, since a derived table and a CTE have no catalog table
-- behind the alias and a join has no single table at all. Reverting only the
-- projection arm's scope reddens this file and leaves
-- `alias_{aggregate,window,scalar_subquery}_declared_types` green, and reverting only
-- those arms does the reverse (both measured) — the split is why they are separate
-- files.
--
-- `alias_same_width` and `alias_qualified` are the polarity pins: both were already
-- correct on the simple-query leg, so an over-correction that decides the type from
-- the wrong side (or from the input's first column) reddens them.
-- @setup
CREATE TABLE mw (s SMALLINT, i INT, b BIGINT);
INSERT INTO mw VALUES (30000, 60000, 5000000000);

-- @test
SELECT m.s + m.i AS alias_qualified,     -- int4  (was text)
       s + b     AS alias_unqualified,   -- int8  (was text; unqualified columns)
       m.b + 0   AS alias_wide_plus_lit, -- int8  (was int4 — the other operand's)
       abs(m.b)  AS alias_abs,           -- int8  (was numeric)
       m.i + m.i AS alias_same_width     -- int4  — polarity pin
FROM mw m;
