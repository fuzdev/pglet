-- The qualified spelling of `select/derived_table_alias_shadows_a_base_table.sql`, and
-- a separate tier of the binding gate rather than the same one: a bare name counts
-- exact claimants across the level's relations (`binding_bare`), while a qualifier picks
-- one relation and asks it for the column (`binding_qualified`). Both read the same
-- `ColumnScope`, so both were wrong while `build_column_scope` resolved a FROM name
-- through `schema.get_table` ahead of the derived-table map — this statement was
-- `42703 column "shadow_qual.z" does not exist` here against 18.6's row.
--
-- Keeping it as its own fixture rather than a second reference in the bare one is
-- deliberate: the two verdicts have moved apart before, and a single `@test` can pin
-- only one error at a time.
-- @setup
CREATE TABLE shadow_qual (a INT, b INT[]);
INSERT INTO shadow_qual VALUES (1, ARRAY[7, 8]);

-- @test
SELECT shadow_qual.z FROM (SELECT 1 AS z) AS shadow_qual;
