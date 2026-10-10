-- A NOT NULL backfill violation (`23502`) outranks an inline `CHECK` violation
-- (`23514`) on the same `ADD COLUMN`: 18.6's `ATRewriteTable` tests the not-null
-- attributes (`tablecmds.c:6465`) before the constraint list (`:6502`), so the textual
-- order of the two clauses does not matter (measured in both spellings).
--
-- **The predicate has to be `c IS NOT NULL`.** With the obvious `CHECK (c > 100)` a
-- NULL backfill makes the predicate NULL, which is no violation on either engine, so
-- the `23502` wins wherever the gate sits and the fixture would assert nothing about
-- ordering. `IS NOT NULL` is the one predicate a NULL actually violates.
--
-- `add_column_backfill_check_without_not_null.sql` is the control: the same statement
-- without the NOT NULL is `23514` on both engines.
-- @expect-error: the not-null backfill is reported before the check constraint
-- @setup
CREATE TABLE anbc (id INT);
INSERT INTO anbc VALUES (1), (2);
-- @test
ALTER TABLE anbc ADD COLUMN c INT NOT NULL DEFAULT NULL CHECK (c IS NOT NULL);
