-- **The discriminating shape for the plan-time binding gate: the table is empty.** A
-- qualifier no relation in scope answers is `42P01` on 18.6 whether or not a row exists,
-- because Postgres raises it in parse analysis. pglet's gate was at eval, so it was
-- **row-count-dependent** — with nothing to evaluate, this statement answered `(0 rows)`.
--
-- Its populated sibling was green either way, which is why the seed is empty: a guard
-- written on the populated spelling proves only that the executor's own tier-3 `42P01`
-- still fires.
-- @setup
CREATE TABLE qbe (id INT, v TEXT);

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
SELECT 1 FROM qbe WHERE zzz.id = 1;
