-- The write half of the same class (`EDGES.md` row 87): `oid → integer` is an
-- **assignment** cast on 18.6's `pg_cast`, so this `INSERT … SELECT` stores a row there
-- and is `42804 column "relid" is of type integer but expression is of type oid` here.
-- Loud and pglet-stricter — no row is written under either engine's rules by accident.
-- @expect-error: 42804 here, `INSERT 0 1` on 18.6 — the pglet-stricter assignment
-- @setup
CREATE TABLE oid_snap (relid INT);
CREATE TABLE oid_src (a INT, CONSTRAINT oid_src_ck CHECK (a > 0));

-- @test
INSERT INTO oid_snap (relid) SELECT conrelid FROM pg_constraint LIMIT 1
