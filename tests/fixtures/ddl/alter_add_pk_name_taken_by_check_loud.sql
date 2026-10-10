-- The other direction: `ADD CONSTRAINT … PRIMARY KEY` under a name an existing CHECK
-- holds. The constraint index clears the *relation* namespace (no index of that name)
-- and then trips the constraint namespace — `index_create` tests them in that order,
-- so this is 42710 and not 42P07.
-- @setup
CREATE TABLE aapc (a INT NOT NULL, CONSTRAINT aapc_ck CHECK (a > 0));

-- @expect-error: constraint aapc_ck for relation aapc already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE aapc ADD CONSTRAINT aapc_ck PRIMARY KEY (a);
