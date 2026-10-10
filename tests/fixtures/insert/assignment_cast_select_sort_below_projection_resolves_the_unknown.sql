-- The **sort-below-projection** spelling of
-- `assignment_cast_select_sort_resolves_the_unknown_loud.sql`, and it is a different
-- plan rather than a different statement: a key on a source column the output drops
-- (`n`) makes `build_sort` put the `Plan::Sort` *under* the `Plan::Project`, so a walk
-- that reads the projection's entries and stops never meets the sort at all. Measured
-- on 18.6 this is the same `42804` its sort-above twin is — the `ORDER BY 1` claims the
-- only entry either way.
--
-- Its accept twin is `assignment_cast_select_sort_below_marks_only_the_keyed_entry.sql`,
-- and between them they say the claim survives the layout **and** stays per-position.
-- @setup
CREATE TABLE acsb (id INT, b BOOL);
CREATE TABLE acsbsrc (n INT);
INSERT INTO acsbsrc VALUES (1), (2);

-- @expect-error: column "b" is of type boolean but expression is of type text (42804)
-- @test
INSERT INTO acsb (b) SELECT 't' FROM acsbsrc ORDER BY 1, n;
