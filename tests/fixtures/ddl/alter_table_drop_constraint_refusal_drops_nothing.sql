-- Documentary: a list whose last action names a kind pglet refuses fails as a whole;
-- 18.6 performs both drops. Any pglet error passes. That the check the first action
-- named is still there is held by a unit test, past the failing statement.
-- @expect-error: the primary key's drop is refused
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE dcrd (a INT PRIMARY KEY, b INT CHECK (b > 0));

-- @test
ALTER TABLE dcrd DROP CONSTRAINT dcrd_b_check, DROP CONSTRAINT dcrd_pkey;
