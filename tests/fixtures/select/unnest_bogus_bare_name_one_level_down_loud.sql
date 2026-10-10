-- The **other** registration site, and the control that says the two are separate code.
--
-- A bogus bare name at an `unnest(…)` level reached through a correlated subquery is
-- judged by `push_level_relations` — the walk that builds a *correlated* sub-plan's
-- scope — which has always registered an `Unnest` with `push_relation`, exhaustively. So
-- this statement was `42703` on both engines before `build_column_scope`'s arm was
-- flipped and is `42703` on both after it, and reverting the flip leaves it green. The
-- shapes that move are the ones at the statement's **own** level
-- (`select/unnest_bogus_bare_name_own_level_loud.sql` and its siblings), which read the
-- other builder.
--
-- The statement is a `DELETE` because the split has a write polarity: the refusal has to
-- arrive at plan time, since a `NOT EXISTS` over an empty array is *true* for every row
-- and an engine that deferred to the eval-time resolver would remove both rows without
-- ever evaluating the bogus reference. Read this fixture narrowly, though: `@expect-error`
-- compares SQLSTATEs and is green for a refusal raised *after* the writes, so what is
-- pinned here is the code and the engine pair, not the row survival. Nothing asserts the
-- survival for this shape today.
--
-- The setup is two statements with no bogus reference in either, so it cannot raise the
-- `42703` the test asserts.
-- @setup
CREATE TABLE pt (id INT, v TEXT);
INSERT INTO pt VALUES (1, 'a'), (2, 'b');

-- @test
-- @expect-error: bare `bogus` names nothing at the unnest level (42703)
DELETE FROM pt WHERE NOT EXISTS (SELECT 1 FROM unnest(ARRAY[]::int[]) AS u(x) WHERE bogus = 1);
