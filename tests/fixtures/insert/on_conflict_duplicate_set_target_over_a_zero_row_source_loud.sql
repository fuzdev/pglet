-- **The member that makes the gate's position payable.** 18.6 refuses the repeated `SET`
-- target from the rewriter, which runs before any row is evaluated, so a row source
-- yielding **zero rows** is refused exactly as a one-row source is. pglet's gate is
-- plan-time and agrees here.
--
-- That agreement is what a "fix" for the row-source residual would cost. pglet reaches a
-- row-source cell's own verdict only per row, at execution, so a gate moved below those
-- verdicts stops firing on this statement — `docs/names.md` § What the refusals do not
-- carry records both halves.
--
-- Its accepting twin is `on_conflict_set_target_over_a_zero_row_source_writes_nothing.sql`.
-- @setup
CREATE TABLE ocdz (a INT UNIQUE, b INT);
CREATE TABLE ocdzsrc (a INT, b INT);

-- @expect-error: multiple assignments to same column "b" (42601)
-- @profile-lock
-- @test
INSERT INTO ocdz SELECT a, b FROM ocdzsrc WHERE false ON CONFLICT (a) DO UPDATE SET b = 1, b = 2;
