-- The second proposal's `WHERE` is false, and 18.6 still refuses it `21000`: the row is
-- locked, and the refusal raised, before the action's `WHERE` is read. A refusal placed
-- after the `WHERE` skips this proposal and leaves the first proposal's write standing.
-- @profile-lock
-- @setup
CREATE TABLE ocsw (id INT PRIMARY KEY, a INT);
INSERT INTO ocsw VALUES (1, 10);

-- @test
-- @expect-error: ON CONFLICT DO UPDATE command cannot affect row a second time (21000)
INSERT INTO ocsw VALUES (1, 1), (1, 2)
ON CONFLICT (id) DO UPDATE SET a = excluded.a WHERE excluded.a = 1;
