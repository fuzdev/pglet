-- The other side of the order: a `SET` target name beats the action's `WHERE`, because
-- `transformOnConflictClause` transforms the target list before the `WHERE` clause. So
-- this is `42703` naming the relation, not the `42P01` its `WHERE` would give — and
-- `insert/on_conflict_action_rhs_precedes_target.sql` is the same statement's other pair,
-- where a right-hand side outranks the target instead.
--
-- Both statements are accepted-and-written here before the action scope closed, so the
-- order was unobservable: it becomes a contract only once both checks exist.
-- @setup
CREATE TABLE octw (n INT PRIMARY KEY, v TEXT);
INSERT INTO octw VALUES (1, 'a');

-- @expect-error: column "nosuchcol" of relation "octw" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the ON CONFLICT action is lowered, before any index selection runs
-- @test
INSERT INTO octw VALUES (2, 'b') ON CONFLICT (n) DO UPDATE SET nosuchcol = 'z' WHERE zzz.v = 'b';
