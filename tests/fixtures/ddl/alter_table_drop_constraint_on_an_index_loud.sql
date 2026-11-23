-- `DROP CONSTRAINT` over an index is the wrong relation kind, `42809`.
-- @expect-error: an index is not a table
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE dcoi (a INT);
CREATE INDEX dcoi_a_idx ON dcoi (a);

-- @test
ALTER TABLE dcoi_a_idx DROP CONSTRAINT x;
