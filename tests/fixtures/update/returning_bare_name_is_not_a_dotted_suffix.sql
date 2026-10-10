-- A bare `RETURNING` name no column carries, over a **single-relation** namespace whose
-- one quoted column name ends with it: `42703` on both engines, and no row.
--
-- This file is the *reject* polarity of the suffix rule. Its accept polarity is
-- `returning_from_dotted_column_is_not_a_suffix_match.sql`, over a multi-relation
-- namespace, where the name has exactly one real claimant and 18.6 answers it; here it
-- has none, and `"s.k"` ending in `.k` must not manufacture one. Neither file can carry
-- the other's polarity: a fixture is one `@test`, and an ordinary bare `RETURNING` name
-- over a single relation is ordinary corpus traffic
-- (`returning_from_unambiguous_names_answer.sql`, and 22 fixture files carry a
-- `RETURNING id`), so a build that refused bare names outright would fail those rather
-- than isolate anything here.
--
-- **Which component this pins, and which it does not.** The verdict is
-- `lower_returning`'s closed `ColumnScope`, which counts zero claimants for `k` at plan
-- time — not a resolver tier. Two
-- walls stand in series on this statement and this file cannot separate them: the
-- executor behind the gate is `returning_column_map`, a `JoinColumnMap` whose bare lookup
-- is exact-only (source read), and a build with *that* resolver reverted to the flat
-- `SimpleColumns` still answers `42703` from the plan-time gate (measured with the
-- resolver reverted, recorded beside its unit test in `helpers.rs`'s
-- `returning_namespace_tests`). So this file asserts the engine's verdict
-- end to end and leaves each wall to that unit test.
--
-- What it covers is the *suffix* shape on the reject side: the target's only near-match
-- for `k` is the tail of `"s.k"`, so any resolution that accepts a suffix answers
-- `TARGETDOTTED` here instead of erroring.
-- `delete/returning_unknown_bare_column_loud.sql` cannot stand in for that — its `@test`
-- names `nosuchcol` over `rub_t(id, b)`, where no column name ends in `.nosuchcol`, so a
-- suffix tier has nothing to match.
-- @setup
CREATE TABLE rbs_t (id INT, "s.k" TEXT, v TEXT);
INSERT INTO rbs_t VALUES (1, 'TARGETDOTTED', 'PLAIN');

-- @test
-- @expect-error: 42703 — no column is named `k`; `"s.k"` is not a suffix match for it
UPDATE rbs_t SET id = rbs_t.id RETURNING k;
