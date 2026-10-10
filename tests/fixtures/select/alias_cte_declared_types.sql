-- The same defect over a **CTE reference**. A `CteScan` reaches the walk through a
-- different branch than a derived table's `SubqueryScan` — its columns come from the
-- CTE's captured descriptors rather than from a sub-plan — and, like the derived
-- table, `c` names nothing in the catalog. Measured against two wrong fixes: keying
-- the fallback on the scan's real table name leaves this red, and so does resolving
-- each qualifier through a FROM alias-to-base-table map (which repairs the join).
--
-- `cte_same_width` is the polarity pin.
-- @setup
CREATE TABLE mw (s SMALLINT, i INT, b BIGINT);
INSERT INTO mw VALUES (30000, 60000, 5000000000);

-- @test
WITH c AS (SELECT s AS cs, i AS ci, b AS cb FROM mw)
SELECT c.cs + c.cb AS cte_qualified,     -- int8 (was text)
       cs + cb     AS cte_unqualified,   -- int8 (was text)
       c.cb + 0    AS cte_wide_plus_lit, -- int8 (was int4)
       c.ci + c.ci AS cte_same_width     -- int4 — polarity pin
FROM c;
