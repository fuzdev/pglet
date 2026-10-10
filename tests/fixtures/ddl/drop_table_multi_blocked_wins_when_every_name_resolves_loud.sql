-- The mirror of `drop_table_multi_blocked_loses_to_later_missing_loud.sql`: same
-- setup, same first name, but the second name resolves — so phase 1 passes and the
-- dependency refusal is the answer (`2BP01`). The pair is what pins the ordering:
-- either fixture alone is satisfied by an engine that always reports its code.
-- @setup
CREATE TABLE dbw (id INT PRIMARY KEY);
CREATE TABLE dbwchild (x INT REFERENCES dbw(id));
CREATE TABLE dbwother (z INT);

-- @expect-error: cannot drop desired object(s) because other objects depend on them
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE dbw, dbwother;
