-- A scalar subquery in the projection is the shape that shows this defect is **not**
-- confined to the extended protocol. The executor's own declared-type walk defers a
-- subquery item to the planner's `Plan::output_columns` rather than re-deriving it,
-- so the planner's wrong answer reached the simple-query `RowDescription` too:
-- measured pre-fix, this query declared `int4` (OID 23) on *both* legs against
-- 18.6's `int8`, where every sibling fixture here diverged on the extended leg
-- alone.
--
-- What actually reddens it on a revert is the **binary decode**, not either
-- declared-type comparison: `int4` over eight bytes is `error deserializing column 0`,
-- and the harness stops a fixture at the value mismatch before it asserts types
-- (measured — both legs then report the fixture as *never reached*). The type legs
-- assert green here on the fixed build, so what they cover is the wrong-yet-
-- width-compatible declaration a decode would swallow.
-- @setup
CREATE TABLE mw (s SMALLINT, i INT, b BIGINT);
INSERT INTO mw VALUES (30000, 60000, 5000000000);

-- @test
-- `sub_agg` is int8; it was int4 on both legs.
SELECT (SELECT max(o.b + 0) FROM mw o) AS sub_agg;
