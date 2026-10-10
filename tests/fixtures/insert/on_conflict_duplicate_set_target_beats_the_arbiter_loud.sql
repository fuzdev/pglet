-- The arbiter is inferred in the **planner** (`infer_arbiter_indexes`) and the duplicate
-- is refused in the rewriter, which runs first — so a `SET` list with a repeat is `42601`
-- even when the conflict target matches no unique index at all, where the clean spelling
-- of the same statement is `42P10`. pglet inferred the arbiter at execution and reported
-- that `42P10`.
-- @setup
CREATE TABLE ocda (a INT PRIMARY KEY, b INT, c INT);
INSERT INTO ocda VALUES (1, 1, 1);

-- @expect-error: multiple assignments to same column "b" (42601)
-- @profile-lock
-- @test
INSERT INTO ocda VALUES (1, 9, 9) ON CONFLICT (c) DO UPDATE SET b = 1, b = 2;
