-- The accept polarity of the action's fourth check: assigning a non-`DEFAULT` value to a
-- `GENERATED ALWAYS` identity column is `428C9`, and `DO UPDATE` never routes through the
-- UPDATE lowering, so the guard has to be applied in the action too.
--
-- It sits **after** the action's three parse-analysis passes, which is what
-- `insert/on_conflict_action_identity_guard_is_last.sql` pins by putting an error in the
-- `WHERE` beside this assignment and getting `42703` instead. Nothing about the three
-- passes predicts that, because the guard concerns the same column an assignment target
-- does; the pair has to be measured.
-- @setup
CREATE TABLE ocia (n INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, v TEXT);
INSERT INTO ocia (v) VALUES ('a');

-- @expect-error: column "n" can only be updated to DEFAULT (428C9)
-- @no-profile-lock: the refusal is raised while the ON CONFLICT action is lowered, before any index selection runs
-- @test
INSERT INTO ocia (v) VALUES ('b') ON CONFLICT (n) DO UPDATE SET n = 5;
