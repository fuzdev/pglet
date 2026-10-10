-- `RETURNING *` over a **CTE reference** source — the namespace's third node kind.
--
-- A `WITH` CTE's columns come from its own projection, and the relation goes into the
-- namespace under the CTE name (or its FROM alias). Its sibling kinds are
-- `returning_from_derived_source_wildcard.sql`, `returning_from_unnest_source_wildcard.sql`
-- and `returning_from_virtual_source_column_ambiguous.sql`; the base-table kind is
-- `returning_from_bare_wildcard.sql`.
-- @setup
CREATE TABLE rcw_t (id INT, v TEXT);
INSERT INTO rcw_t VALUES (1, 'T');

-- @test
WITH c AS (SELECT 5 AS cv) UPDATE rcw_t SET id = 1 FROM c RETURNING *;
