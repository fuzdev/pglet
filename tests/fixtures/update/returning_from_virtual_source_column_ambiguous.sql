-- A bare `RETURNING` name a **catalog view** source also carries — `42702` on 18.6.
--
-- The sibling of `returning_from_source_column_ambiguous.sql`, and the reason that one
-- alone was not enough: the ambiguity verdict is decided by counting the claimants the
-- statement's `FROM` introduces, and the walk that collects them (`plan_relations`) had
-- arms for a base table, a CTE and a derived table only. A source that lowered to any
-- *other* node kind contributed **no claimant**, the bare name looked unambiguous, and
-- the statement answered the target's column with no error — the very silent wrong the
-- widened namespace exists to remove, surviving on the kinds the walk could not see.
--
-- Both engines carry `table_name` on `information_schema.tables`, so the target's own
-- `table_name` column is what makes the reference ambiguous. Its accepting polarity is
-- `returning_from_virtual_source_unambiguous_name_answers.sql`; a fix that refused every
-- bare name over a virtual source reddens there.
-- @setup
CREATE TABLE rvt (table_name TEXT, x INT);
INSERT INTO rvt VALUES ('rvt', 0);

-- @test
-- @expect-error: 42702 — the catalog-view source claims `table_name` too
UPDATE rvt SET x = 1 FROM information_schema.tables c
  WHERE c.table_name = rvt.table_name RETURNING table_name;
