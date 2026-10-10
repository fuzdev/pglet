-- **The control that says the plan-time close is what moved, not the error.** This is
-- `insert/on_conflict_action_binds_at_plan_time.sql` with a **conflicting** key, so the
-- action is evaluated and the eval-time resolution raises the same `42703` whether or not
-- the scope is closed. It was green before the close and is green after.
--
-- The pair is the discriminator: a guard written only on this polarity proves nothing
-- about the shape that actually wrote a row, which is the one that hits no conflict.
-- @setup
CREATE TABLE ocmc (n INT PRIMARY KEY, v TEXT);
INSERT INTO ocmc VALUES (1, 'a');

-- @expect-error: column "nosuchcol" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the ON CONFLICT action is lowered, before any index selection runs
-- @test
INSERT INTO ocmc VALUES (1, 'b') ON CONFLICT (n) DO UPDATE SET v = nosuchcol;
