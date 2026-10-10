-- `alias.*` over a shadowing derived table. A third reader, and it fails in two steps
-- rather than one: `expand_wildcards` reads the derived-table map (which is keyed by
-- effective FROM name and was always right), so the star expanded to `shadow_star.z`
-- correctly — and the binding gate, reading a `ColumnScope` that had resolved the same
-- name to the *base table*, then refused the column the expansion had just produced.
-- `42703 column "shadow_star.z" does not exist` here against 18.6's row.
--
-- That split is the reason this is worth pinning next to the plain qualified reference:
-- the two maps disagreed about what one FROM name means, and the expansion's map was
-- the one that was right.
-- @setup
CREATE TABLE shadow_star (a INT, b INT[]);
INSERT INTO shadow_star VALUES (1, ARRAY[7, 8]);

-- @test
SELECT shadow_star.* FROM (SELECT 1 AS z, 2 AS w) AS shadow_star;
