-- Documentary: dropping a `PRIMARY KEY` is pglet's `0A000`, a loud refusal where 18.6
-- drops the constraint and its index. 18.6 accepts, so any pglet error passes; the
-- record is the refusal, not its code.
-- @expect-error: pglet does not drop a primary key constraint
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE dcpk (a INT PRIMARY KEY);

-- @test
ALTER TABLE dcpk DROP CONSTRAINT dcpk_pkey;
