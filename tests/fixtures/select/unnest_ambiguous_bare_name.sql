-- A bare name claimed by **both** a base table and a FROM-clause `unnest(…)`. The
-- claimant count is a scope fact, so 18.6 raises `42702` in parse analysis
-- (`colNameToVar`) whether or not a row exists — and pglet's plan-time binding gate now
-- does too, because `build_column_scope` registers the unnest's own columns.
--
-- Before that registration the unnest contributed no claimant, so the gate saw one
-- exact claimant and bound the table's column; the ambiguity was then noticed only by
-- the eval-time resolver, i.e. only when a row reached it. The **empty** array is what
-- makes that visible: this exact statement answered `(0 rows)` here against 18.6's
-- `42702`, while the same statement over `ARRAY[7]` was already `42702` on both. So the
-- closure is row-count-dependent in the direction that hides it, and an array with an
-- element would pin nothing.
--
-- Measured at Parse on both engines (raw v3 `Parse` + `Sync`, no `Bind`): the error
-- arrives before `ParseComplete`, so it is not the executor's.
--
-- The setup deliberately cannot itself raise `42702` — it is two DDL statements and an
-- INSERT with no ambiguous reference in it, so a green run here is the `@test`'s verdict.
-- @setup
CREATE TABLE amb_un (id INT, x INT);
INSERT INTO amb_un VALUES (1, 10), (2, 20);

-- @test
-- @expect-error: bare `id` is claimed by both `amb_un` and the unnest (42702)
SELECT id FROM amb_un, unnest(ARRAY[]::int[]) AS u(id);
