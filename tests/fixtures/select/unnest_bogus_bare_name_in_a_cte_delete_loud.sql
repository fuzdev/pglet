-- **The write polarity of the exhaustiveness split: this shape destroyed rows.** An
-- unreferenced CTE's body never executes, so its bogus name never reaches the eval-time
-- resolver, and with the unnest registered non-exhaustively the plan-time gate answered
-- `Uncertain` — nothing judged the reference at all, and the `DELETE` then ran: measured
-- on the pre-flip build, no error and **0 rows left** of two, against 18.6's `42703` with
-- both rows surviving. With `build_column_scope` registering the unnest exhaustively the
-- gate refuses at plan time and both rows survive here too.
--
-- This is the one shape in the family where the flip stops a data loss rather than a
-- wrong answer, which is why it is pinned separately from the read-polarity rejects
-- (`select/unnest_bogus_bare_name_own_level_loud.sql` and siblings). Read it narrowly,
-- as `select/unnest_bogus_bare_name_one_level_down_loud.sql` says of its own shape: the
-- harness runs under `BEGIN … ROLLBACK` and `@expect-error` compares SQLSTATEs, so what
-- is pinned is the code and the engine pair, not the row survival. Emptiness is not
-- load-bearing for this polarity — nothing about the statement depends on the CTE's row
-- count — and the empty spelling is kept so this fixture differs from the read one in
-- the statement kind alone.
--
-- The setup is two statements with no bogus reference in either, so it cannot raise the
-- `42703` the test asserts.
-- @setup
CREATE TABLE cdel (id INT, v TEXT);
INSERT INTO cdel VALUES (1, 'a'), (2, 'b');

-- @test
-- @expect-error: bare `bogus` names nothing at the CTE's unnest level (42703)
WITH c AS (SELECT bogus FROM unnest(ARRAY[]::int[]) AS u(x)) DELETE FROM cdel;
