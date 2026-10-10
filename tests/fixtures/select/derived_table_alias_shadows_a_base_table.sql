-- A FROM item's alias makes that item what its name binds to for the whole query level.
-- A base table that happens to be *called* the same thing is not in scope under that
-- name at all — `addRangeTableEntry` registers the derived table's eref and nothing
-- else, so the only columns `shadow_src` carries here are `z`.
--
-- `build_column_scope` used to decide which columns a FROM name carries by asking
-- `schema.get_table` **first** and the derived-table / CTE map only on its miss, so any
-- base table sharing the spelling won over the FROM item that actually binds the
-- name: the scope registered `shadow_src(a, b)` — exhaustively — and the binding
-- gate answered `42703 column "z" does not exist` to a statement 18.6 answers. A
-- **false rejection**, at any row count and with no `unnest` in sight; a CTE or derived
-- table named for the table it derives from is ordinary SQL. The lookup is ordered by
-- FROM-item **kind** now.
--
-- The base table is deliberately given columns the derived table does not have and vice
-- versa, so a scope built from the wrong relation cannot accidentally answer: `z` exists
-- only on the derived table, and the reject-polarity guards
-- `select/derived_table_alias_shadow_hides_base_columns.sql` and its qualified sibling
-- assert the other direction — `a` and `b` are *gone* under this name, not merely
-- outranked.
-- @setup
CREATE TABLE shadow_src (a INT, b INT[]);
INSERT INTO shadow_src VALUES (1, ARRAY[7, 8]);

-- @test
SELECT z FROM (SELECT 1 AS z) AS shadow_src;
