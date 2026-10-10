-- The converse of `set_check_passes_on_the_old_row.sql`: over `(1, 2)` the new row is
-- `(2, 2)` — `b = a + 1` reads the old `a` — and violates the `CHECK`. The in-place
-- reading `(2, 3)` satisfies it; pglet wrote it.
-- @setup
CREATE TABLE socf (a INT, b INT, CHECK (a <> b));
INSERT INTO socf VALUES (1, 2);

-- @expect-error: new row violates check constraint (23514)
-- @test
UPDATE socf SET a = b, b = a + 1;
