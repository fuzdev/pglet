-- **pglet-stricter, and recorded rather than closed** (`EDGES.md` row 87). 18.6 carries
-- `oid` through `select_common_type` — `coalesce(conrelid, 0)` is an `oid` there, because
-- `pg_cast` marks `integer → oid` implicit and `oid → integer` assignment — while pglet
-- models `oid` with comparison rules and no cast rows at all, so the fold has no common
-- type and refuses `42804`. The refusal is loud and cannot write, which is why it is
-- recorded; this fixture pins that it stays loud, whatever the fold later learns.
--
-- The polarity that separates it from a bug: `nullif(conrelid, 0)` **answers** here,
-- resolving through the comparison rulebook instead — see the row.
-- @expect-error: 42804 here, an `oid` on 18.6 — the pglet-stricter fold
-- @setup
CREATE TABLE oid_snap (relid INT);
CREATE TABLE oid_src (a INT, CONSTRAINT oid_src_ck CHECK (a > 0));

-- @test
SELECT coalesce(conrelid, 0) FROM pg_constraint LIMIT 1
