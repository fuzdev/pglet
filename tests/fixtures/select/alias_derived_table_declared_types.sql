-- The same defect over a **derived table**. Nothing in the catalog answers to `x`,
-- so this shape is the discriminator against the wrong fix the sibling
-- `alias_projection_declared_types` admits: keying the walk's fallback on the scan's
-- *real table name* fixes `FROM mw m` and leaves this red (measured), because a
-- `SubqueryScan` has no real table name — its alias is the only name it has. An
-- alias-to-base-table map leaves it red too. Resolving against the plan node's input
-- descriptors is what covers all of them.
--
-- `derived_same_width` is the polarity pin (int4 in, int4 out); `derived_unqualified`
-- shows the alias need not be written on the column for the shape to reach the walk.
-- @setup
CREATE TABLE mw (s SMALLINT, i INT, b BIGINT);
INSERT INTO mw VALUES (30000, 60000, 5000000000);

-- @test
SELECT x.a + x.d AS derived_qualified,     -- int8 (was text)
       a + d     AS derived_unqualified,   -- int8 (was text)
       x.d + 0   AS derived_wide_plus_lit, -- int8 (was int4)
       x.c + x.c AS derived_same_width     -- int4 — polarity pin
FROM (SELECT s AS a, i AS c, b AS d FROM mw) x;
