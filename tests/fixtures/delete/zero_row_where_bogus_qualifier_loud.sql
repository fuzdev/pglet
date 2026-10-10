-- A `DELETE` whose `WHERE` matches nothing never evaluates it, so a bogus qualifier there
-- was `DELETE 0` — silent success. The target alone is the whole namespace a `DELETE`
-- clause may name (this parser has no `USING`, and there is no enclosing level), so the
-- scope closes and the verdict stops depending on a row.
--
-- The seed is deliberately **empty**: the populated spelling was already loud, from the
-- executor's own tier-3 `42P01`, so a guard written on it says nothing about this gate.
-- `update/zero_row_set_bogus_qualifier_loud.sql` is the same gate on the other DML path,
-- whose namespace additionally carries the statement's `FROM`.
-- @setup
CREATE TABLE zdw (id INT, v TEXT);

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
DELETE FROM zdw WHERE zzz.id = 1;
