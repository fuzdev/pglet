-- The second verdict a row-source cell carries, under the same control: a bare name the
-- statement's namespace does not hold is `42703` on both engines when no repeat stands in
-- front of it. Its repeated spelling is the other half of the split
-- `on_conflict_single_set_target_yields_to_the_cell_cast_loud.sql` describes, and is
-- unfixturable for the same reason.
-- @setup
CREATE TABLE ocdname (a INT UNIQUE, b INT, bo BOOL);
INSERT INTO ocdname VALUES (1, 1, true);

-- @expect-error: column "nosuchcol" does not exist (42703)
-- @profile-lock
-- @test
INSERT INTO ocdname VALUES (5, nosuchcol, true) ON CONFLICT (a) DO UPDATE SET b = 1;
