-- 42P07 is bare too, and pglet holds the relation name — another negative.
-- @setup
CREATE TABLE t_dup (x INT);

-- @test
-- @expect-error: the relation already exists
-- @expect-message
CREATE TABLE t_dup (y INT);
