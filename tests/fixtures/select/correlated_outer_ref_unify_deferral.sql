-- **The unification deferral**, which is what makes the correlated re-resolution safe
-- to run twice. `CASE` / `COALESCE` / `greatest` pick a common type from their
-- siblings, and a correlated reference the subquery's own scope cannot type would
-- otherwise be *excluded* from that choice — so the first pass picks a target from the
-- remaining siblings and casts the correlated operand to it, permanently. The
-- enclosing level's pass then sees a `Cast` and has nothing left to fix.
--
-- So a branch that is a column the scope cannot type makes the whole unification
-- decline (`is_untyped_column`), leaving the expression untouched for the level that
-- can type it.
--
-- Both legs below are wrong without that. `greatest(uo.oamt, ui.iamt)` picks `int4`
-- from the inner sibling, narrows the correlated `float8` 1.5 to 2, and the comparison
-- **silently returns no rows** — no error, on a query Postgres answers. The `COALESCE`
-- leg picks `int4` the same way and then fails `22P02` parsing `'1.5'` as an integer.
-- @setup
CREATE TABLE uo (okey INT, oamt FLOAT8);
CREATE TABLE ui (ikey INT, iamt INT);
INSERT INTO uo VALUES (1, 1.5), (2, 9.5);
INSERT INTO ui VALUES (1, 1);
-- @test
SELECT okey
  FROM uo
 WHERE EXISTS (SELECT 1 FROM ui WHERE greatest(uo.oamt, ui.iamt) = 1.5)
    OR EXISTS (SELECT 1 FROM ui WHERE COALESCE(uo.oamt, ui.iamt, '1.5') = 9.5)
 ORDER BY okey;
