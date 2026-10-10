-- A `PRIMARY KEY` / `UNIQUE` naming a column the table lacks is raised during *parse
-- analysis* (`transformIndexConstraint`, `parse_utilcmd.c`), so like the second-primary-key
-- 42P16 beside it, it outranks every execution-time error in the same statement: this
-- CREATE TABLE also carries a duplicate CHECK name (42710), an ill-typed CHECK (42883),
-- and a foreign key to a missing table (42P01), and Postgres reports none of them.
--
-- This one is not only about which error wins. pglet raised the 42703 from inside the
-- index-build loop, *after* `schema.create_table` — and the autocommit path rolls back
-- storage while leaving the catalog mutated, so the failed statement left a durable
-- phantom table: `information_schema.tables` listed it, reads answered 42P01, and the
-- same `CREATE TABLE` was 42P07 forever. The BEGIN…ROLLBACK the harness wraps setup in
-- hides that half; `a_failed_create_table_leaves_no_phantom` (crates/pglet) is its guard.
-- @expect-error: column b named in key does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctkc (a INT NOT NULL,
  CONSTRAINT dup CHECK (a > 0), CONSTRAINT dup CHECK (a > 1),
  CHECK (a LIKE 'x'),
  FOREIGN KEY (a) REFERENCES ctkc_no_such_parent (x),
  PRIMARY KEY (b));
