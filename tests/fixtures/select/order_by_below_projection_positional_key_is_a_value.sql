-- A sort-below-projection layout carries an output-naming key down as the projection
-- expression that produced it, and a **bare integer literal** is also the plan's
-- spelling of a positional `ORDER BY <n>` — so carrying one raw makes the Sort read
-- base column `n` of the pre-projection row instead of evaluating the constant.
--
-- Here `ORDER BY 1` names the constant entry `2`, and the key on the dropped base column
-- `a` is what puts the Sort below the projection. Read raw, the carried `2` selected
-- base column 2 (`b`) and the rows came back in `b` order — `10, 20, 30` against 18.6's
-- `10, 30, 20`, **with no error on either engine**. Its loud sibling is the same rewrite
-- over a source too narrow for the ordinal, which was `42P10`
-- (`insert/assignment_cast_select_sort_below_marks_only_the_keyed_entry.sql`).
-- @setup
CREATE TABLE obpv (a INT, b INT);
INSERT INTO obpv VALUES (1, 10), (2, 30), (3, 20);
-- @test
SELECT 2 AS z, b FROM obpv ORDER BY 1, a;
