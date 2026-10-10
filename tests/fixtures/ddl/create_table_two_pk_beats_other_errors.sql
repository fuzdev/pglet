-- 42P16 is raised during *parse analysis* (`transformIndexConstraints`), so it
-- outranks every execution-time error in the same statement: this CREATE TABLE also
-- carries a duplicate CHECK name (42710) and a foreign key to a missing table
-- (42P01), and Postgres reports neither.
-- @expect-error: multiple primary keys for table cttb are not allowed
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE cttb (a INT NOT NULL, b INT NOT NULL,
  CONSTRAINT dup CHECK (a > 0), CONSTRAINT dup CHECK (b > 0),
  FOREIGN KEY (a) REFERENCES cttb_no_such_parent (x),
  PRIMARY KEY (a), PRIMARY KEY (b));
