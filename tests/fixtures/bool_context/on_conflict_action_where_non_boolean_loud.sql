-- The action's `WHERE` silently skipped the update. 18.6 spells this clause
-- `"WHERE"` too.
-- @setup
CREATE TABLE bcocn (k INT PRIMARY KEY, v INT);
INSERT INTO bcocn VALUES (1, 1);
-- @expect-error: argument of WHERE must be type boolean
-- @no-profile-lock: the refusal is the planner's clause-root gate, raised before
-- index selection runs at all, so both profiles reach it identically.
-- @test
INSERT INTO bcocn VALUES (1, 2) ON CONFLICT (k) DO UPDATE SET v = 3 WHERE bcocn.v;
