-- The shadow's blast radius reaches the *type* a reference carries, not only whether it
-- binds — and this is the spelling where the two are separable, because the base table
-- carries a column of the **same name and a different type**. A scope resolving
-- `shadow_coerce` to the base table finds a `z`, binds it, and hands
-- `ColumnScope::coercion_type` `text`; the quoted `'5'` in the predicate then stays a text
-- constant instead of becoming the int4 one the derived table's `z` calls for. Measured
-- pre-repair, that is exactly what happened: `42883 operator does not exist: integer =
-- text` — the eval-time rulebook comparing the derived table's int4 row value against a
-- literal the plan-time pass had typed off the base table — against 18.6's row.
--
-- Every other fixture in this group gives the base table columns the derived table does
-- not have, which makes a mis-resolution fail the **binding** gate with `42703`. Here the
-- name resolves and only its type is wrong, which is the polarity `coercion_type` exists
-- for and the one a fixture written for the binding gate cannot reach.
-- @setup
CREATE TABLE shadow_coerce (z TEXT, b INT[]);
INSERT INTO shadow_coerce VALUES ('x', ARRAY[7, 8]);

-- @test
SELECT z FROM (SELECT 5 AS z) AS shadow_coerce WHERE z = '5';
