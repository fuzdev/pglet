-- **The discriminating shape for the whole `ON CONFLICT` action scope: the INSERT hits no
-- conflict.** The action then never evaluates, so every eval-time gate over it is silent
-- and the row is written — `42703` on 18.6, `INSERT 0 1` here, the same statement erroring
-- or writing depending on whether the row already existed. The action's namespace is
-- complete by construction (the target plus `excluded`, no enclosing level), so the scope
-- is closed at plan time and the verdict no longer depends on a row.
--
-- The conflicting sibling `insert/on_conflict_action_missing_column_on_conflict.sql` is
-- green either way, which is exactly why this one is written with a **new** key.
-- @setup
CREATE TABLE ocpt (n INT PRIMARY KEY, v TEXT);
INSERT INTO ocpt VALUES (1, 'a');

-- @expect-error: column "nosuchcol" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the ON CONFLICT action is lowered, before any index selection runs
-- @test
INSERT INTO ocpt VALUES (2, 'b') ON CONFLICT (n) DO UPDATE SET v = nosuchcol;
