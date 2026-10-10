-- **The pair no per-assignment loop can produce.** `transformUpdateTargetList` transforms
-- *every* right-hand side before assigning any of them to a column, so the **second**
-- assignment's bogus qualifier beats the **first** assignment's non-existent target:
-- `42P01`, not the `42703` a loop that finished assignment 1 before starting assignment 2
-- would give. Measured on 18.6; pglet used to write the row on this statement, having
-- gated neither.
--
-- The three neighbours that fix the order in place, each measured on both engines and each
-- a different pair: `SET nosuchcol = 'z' WHERE zzz.v = 'b'` is `42703` (a target beats the
-- action's `WHERE`), `SET v = zzz.v WHERE nosuchcol = 1` is `42P01` (a right-hand side
-- beats it too), and over a `GENERATED ALWAYS` `n`, `SET n = 5 WHERE nosuchcol = 1` is
-- `42703` — so the identity guard is last of the four, a rewriter check rather than a
-- parse-analysis one.
-- @setup
CREATE TABLE ocrt (n INT PRIMARY KEY, v TEXT);
INSERT INTO ocrt VALUES (1, 'a');

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @no-profile-lock: the refusal is raised while the ON CONFLICT action is lowered, before any index selection runs
-- @test
INSERT INTO ocrt VALUES (2, 'b') ON CONFLICT (n) DO UPDATE SET nosuchcol = 'y', v = zzz.v;
