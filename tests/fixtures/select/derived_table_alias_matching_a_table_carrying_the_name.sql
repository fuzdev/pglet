-- The control that identifies the defect as *which relation the name resolves to* rather
-- than *whether the name resolves*. Same collision as
-- `select/derived_table_alias_shadows_a_base_table.sql` — a derived table aliased to a
-- base table's name — except the referenced column exists on **both** relations, so the
-- wrong lookup order answered too. Green on both engines before and after the repair.
--
-- What it guards is the *value*, which the sibling fixtures cannot: they assert that a
-- shadowed name binds, and a repair that bound it to the wrong relation would still have
-- to fail their `42703`. Here both relations answer, so only the number separates them —
-- `1` is the derived table's `a`, `9` is the base table's, and an engine that resolved the
-- shadow backwards is green on every other fixture in this group and red on this one.
-- (The base-table arm itself needs no guard here; the rest of the corpus is that guard.)
-- @setup
CREATE TABLE shadow_both (a INT, b INT[]);
INSERT INTO shadow_both VALUES (9, ARRAY[7, 8]);

-- @test
SELECT a FROM (SELECT 1 AS a) AS shadow_both;
