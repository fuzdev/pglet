-- A plain `CREATE INDEX` is in that namespace as well: naming one after a table is
-- 42P07.
-- @setup
CREATE TABLE cint (a INT);
CREATE TABLE cint_other (b INT);

-- @expect-error: relation cint_other already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE INDEX cint_other ON cint (a);
