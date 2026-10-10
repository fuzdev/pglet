-- A data-modifying CTE's list renames its `RETURNING` columns. This reference kind
-- takes a different route from a read-only one: the relation is materialized, so the
-- rename is applied to the plan-side `CteScan` and to the materialized rows separately.
-- @setup
CREATE TABLE cld (a INT);
INSERT INTO cld VALUES (7), (8);
-- @test
WITH dd(gone) AS (DELETE FROM cld WHERE a = 7 RETURNING a) SELECT gone FROM dd;
