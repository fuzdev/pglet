-- The first column waits on the enclosing column's type, and its conversion is still
-- reported ahead of the second column's.
-- @setup
CREATE TABLE scc (i INT);
INSERT INTO scc VALUES (7);
-- @expect-error: invalid input syntax for type integer: "y"
-- @expect-message
-- @test
SELECT count(*) FROM scc WHERE EXISTS (SELECT i, 'x' EXCEPT SELECT 'y', 2);
