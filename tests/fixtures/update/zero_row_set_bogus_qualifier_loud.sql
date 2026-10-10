-- An `UPDATE` whose `WHERE` matches nothing never evaluates its `SET` list, so a bogus
-- qualifier there was `UPDATE 0` — silent success. 18.6 raises `42P01` in parse analysis,
-- whatever the `WHERE` matches.
--
-- The target plus its `FROM` is the whole namespace an `UPDATE` clause may name — no
-- enclosing level to chain to — so the scope closes and the verdict stops depending on a
-- row. `update/zero_row_set_unknown_column_loud.sql` is the `42703` half of the same
-- gate, and `update/unaliased_bogus_qualifier_loud.sql` the populated spelling that was
-- already loud.
-- @setup
CREATE TABLE zrs (id INT, v TEXT);
INSERT INTO zrs VALUES (1, 'a');

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
UPDATE zrs SET v = zzz.v WHERE 1 = 0;
