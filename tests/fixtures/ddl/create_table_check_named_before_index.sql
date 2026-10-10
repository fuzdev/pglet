-- The other side of that split: in `CREATE TABLE` the CHECK is named first, so its
-- auto-name `ctcni_c_check` is already taken when the declared `UNIQUE` of the same name
-- reaches `index_create` — 42710. The identical column definition succeeds under
-- `ADD COLUMN` (see `addcol_index_named_before_check.sql`). Two routes, two orders,
-- both observable.
-- @expect-error: constraint ctcni_c_check for relation ctcni already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctcni (c INT CONSTRAINT ctcni_c_check UNIQUE CHECK (c > 0));
