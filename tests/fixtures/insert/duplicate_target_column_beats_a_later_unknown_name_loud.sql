-- The two verdicts a column list produces **interleave**, one item at a time:
-- `checkInsertTargets` resolves a name and tests it against the columns it has already
-- claimed before it reads the next one, so a repeat at item 2 outranks an unknown name at
-- item 3. pglet ran the resolution as a whole-list pass and reported the `42703`.
--
-- Its inverse is `unknown_target_column_beats_a_later_duplicate_loud.sql`, and the pair
-- is what makes the interleave observable: neither position alone distinguishes an
-- interleaved walk from two whole-list passes. The `UPDATE` twin of the pair
-- (`update/unknown_column_beats_the_duplicate_set_target_loud.sql`) reports the
-- unknown name from **either** position, which is why a helper shared across the
-- families cannot carry the order.
-- @setup
CREATE TABLE dtci (a INT, b INT);

-- @expect-error: column "a" specified more than once (42701)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtci (a, a, zz) VALUES (1, 2, 3);
