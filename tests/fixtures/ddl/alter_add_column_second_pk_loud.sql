-- The third route to an index build — an inline PRIMARY KEY on an added column —
-- carries the same 42P16 gate. Each route reaches `index_check_primary_key` in
-- Postgres, so each needs the check here.
-- @setup
CREATE TABLE aacp (a INT NOT NULL PRIMARY KEY);

-- @expect-error: multiple primary keys for table aacp are not allowed
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE aacp ADD COLUMN c INT NOT NULL PRIMARY KEY;
