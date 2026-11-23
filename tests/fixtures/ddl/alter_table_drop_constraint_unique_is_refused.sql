-- Documentary: dropping a `UNIQUE` constraint is pglet's `0A000`, a loud refusal where
-- 18.6 drops the constraint and its index — even under `IF EXISTS`, which skips only a
-- name no constraint holds. 18.6 accepts, so any pglet error passes.
-- @expect-error: pglet does not drop a unique constraint
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE dcuq (a INT UNIQUE);

-- @test
ALTER TABLE dcuq DROP CONSTRAINT IF EXISTS dcuq_a_key;
