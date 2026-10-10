-- Two FOREIGN KEY constraints declared with the same name on one table is 42710
-- (`constraint "dup" for relation "ch" already exists`). Without the check pglet
-- accepted both, and a later `DROP COLUMN a` then shed *both* — silently
-- un-enforcing the key on `b`, which the drop never named.
-- @setup
CREATE TABLE ctdf_p1 (id INT PRIMARY KEY);
CREATE TABLE ctdf_p2 (id INT PRIMARY KEY);

-- @expect-error: constraint dup for relation ctdf_ch already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctdf_ch (a INT, b INT,
  CONSTRAINT dup FOREIGN KEY (a) REFERENCES ctdf_p1(id),
  CONSTRAINT dup FOREIGN KEY (b) REFERENCES ctdf_p2(id));
