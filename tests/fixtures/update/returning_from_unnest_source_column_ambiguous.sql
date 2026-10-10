-- A bare `RETURNING` name a **`FROM`-clause `unnest(…)`** source also carries —
-- `42702` on 18.6. The second kind `push_plan_relations` could not see; see
-- `returning_from_virtual_source_column_ambiguous.sql` for the class.
--
-- This one needed the *column name* fixed before the claimant could be counted at all.
-- `FROM unnest(a) AS v` names its single output column `v` in Postgres
-- (`chooseScalarFunctionAlias`: one function in the RTE and an alias, so the alias
-- names the column as well as the relation); pglet named it `unnest`, so the relation
-- claimed nothing called `v` and the reference resolved to the target's column and
-- answered a row. Both halves are needed: registering the relation while it still named
-- its column `unnest` leaves this file red.
--
-- The accepting polarity is `returning_from_unnest_source_unambiguous_name_answers.sql`.
-- The column naming itself is pinned from the plain-`SELECT` side by
-- `array/unnest_alias_names_the_column.sql` and its two negative siblings.
-- @setup
CREATE TABLE run_t (id INT, v TEXT);
INSERT INTO run_t VALUES (1, 'TARGET');

-- @test
-- @expect-error: 42702 — the unnest source's column is named after its alias, and the target has a `v` too
UPDATE run_t SET id = 1 FROM unnest(ARRAY['x', 'y']) AS v RETURNING v;
