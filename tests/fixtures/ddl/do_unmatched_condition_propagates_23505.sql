-- An error the handler does **not** match propagates rather than being swallowed:
-- `30` is inserted, then re-inserted to violate the key, and a `duplicate_table`
-- handler does not catch `23505`.
--
-- The sibling `do_block_unmatched_error_loud.sql` locks the same rule over a
-- different pair (`42P01` against a `duplicate_object` handler), which is what
-- keeps this from resting on one condition name.
--
-- What it deliberately does **not** claim is what the body left behind. An
-- `@expect-error` fixture compares the SQLSTATE and nothing else, so the rows are
-- unassertable here; the embed test `an_unmatched_do_error_leaves_no_rows_behind`
-- carries that half, where the statement can fail and the table still be read.
-- @no-profile-lock: the error is the DO body's own 23505, raised by the unique-index
-- probe that both profiles run (PK/UNIQUE enforcement is independent of query index
-- acceleration), and the `@test` selects nothing — there is no read for a plan to
-- differ on.
-- @setup
CREATE TABLE do_unmatched (id INT PRIMARY KEY);
-- @test
-- @expect-error: 23505 is not caught by a duplicate_table handler, so it propagates
DO $$ BEGIN
    INSERT INTO do_unmatched VALUES (30);
    INSERT INTO do_unmatched VALUES (30);
EXCEPTION WHEN duplicate_table THEN NULL;
END $$;
