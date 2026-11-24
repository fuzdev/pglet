-- `to_regclass(text)` answers the relation a name resolves to, rendered as `::regclass`
-- renders it: folded unless quoted, whitespace around it skipped, `public.` stripped, and
-- a quoted name quoted again. A table, an index and a sequence are relations, and so is a
-- catalog relation pglet serves, bare or under `pg_catalog`. pglet's value is the name
-- typed `text` where 18.6's is an OID typed `regclass` (`EDGES.md` row 85), so each
-- column is compared through `::text`.
-- @setup
CREATE TABLE trc_plain (id INT);
CREATE TABLE "TrcOdd" (id INT PRIMARY KEY);
CREATE SEQUENCE trc_seq;
-- @test
SELECT to_regclass('trc_plain')::text AS plain,
       to_regclass('TRC_PLAIN')::text AS folded,
       to_regclass('"TrcOdd"')::text AS quoted,
       to_regclass('  public . trc_plain  ')::text AS qualified,
       to_regclass('"TrcOdd_pkey"')::text AS idx,
       to_regclass('trc_seq')::text AS seq,
       to_regclass('pg_class')::text AS cat,
       to_regclass('pg_catalog.pg_class')::text AS cat_qualified;
