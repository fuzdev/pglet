-- **Set equality is still equality**, and this is the control that stops the set rule
-- from degrading into subset matching: `(a, a)` collapses to `{a}`, which is not the
-- `{a, b}` the index carries, so the statement is `42P10` on both engines. It was `42P10`
-- here before the set rule too — by the wrong reasoning, the two lists differing in
-- length — so only a fix that keeps it red for the right reason is complete.
--
-- Its accept twin is `on_conflict_arbiter_repeat_within_a_composite.sql`, the same
-- repeated `a` with the missing `b` restored.
-- @setup
CREATE TABLE oas (a INT, b INT, UNIQUE (a, b));
INSERT INTO oas VALUES (1, 1);

-- @expect-error: there is no unique or exclusion constraint matching the ON CONFLICT specification (42P10)
-- @profile-lock
-- @test
INSERT INTO oas VALUES (1, 1) ON CONFLICT (a, a) DO NOTHING;
