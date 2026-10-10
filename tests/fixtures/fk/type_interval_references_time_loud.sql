-- A foreign key 18.6 cannot implement — referencing type `interval`, referenced key
-- `time`. One-way too: `time → interval` is implicit, so `time` referencing `interval` is accepted, while
-- `time_ops` holds no `interval` member and `interval → time` is no implicit cast.
-- @setup
CREATE TABLE ftp (k TIME PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: interval and time
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k INTERVAL REFERENCES ftp);
