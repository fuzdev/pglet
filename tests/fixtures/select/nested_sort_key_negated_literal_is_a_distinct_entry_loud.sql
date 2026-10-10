-- @setup
CREATE TABLE snl1 (c INT, x INT);
CREATE TABLE snl2 (k INT);
INSERT INTO snl1 VALUES (1, 10), (2, 20);
INSERT INTO snl2 VALUES (1), (2);

-- The control for `nested_sort_key_negated_zero_is_one_entry.sql`: folding unary minus
-- into the constant does **not** make every `x`/`-x` pair one entry — it makes them one
-- entry exactly when the folded values are equal. `1` and `-1` are two distinct constants
-- to `datumIsEqual`, so the key is ambiguous: `42702` on both engines, before and after
-- the fold. Without this pair a fold that collapsed the sign away would pass the zero
-- fixture and silently take the first entry here — the shape whose `DELETE` spelling
-- removed every row when a claimant count was got wrong before.
--
-- The subquery is `WHERE false`, so nothing is evaluated; the key must be nested, since a
-- top-level bare sort key is exempt from the plan-time gate.
-- @expect-error: ORDER BY "q" is ambiguous (42702)
-- @test
SELECT k FROM snl2 WHERE EXISTS (SELECT 1 AS q, -1 AS q FROM snl1 WHERE false ORDER BY q);
