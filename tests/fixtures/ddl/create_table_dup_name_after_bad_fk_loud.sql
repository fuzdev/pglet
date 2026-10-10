-- Error order, half one: the constraints are resolved in source order, so a missing
-- parent table on the **first** foreign key is reported (42P01) even though a later
-- constraint duplicates its name. Adding the name gate had to not jump the queue.
-- @setup
CREATE TABLE ctdn_p1 (id INT PRIMARY KEY);

-- @expect-error: relation ctdn_missing does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctdn_ch (a INT, b INT,
  CONSTRAINT dup FOREIGN KEY (a) REFERENCES ctdn_missing(id),
  CONSTRAINT dup FOREIGN KEY (b) REFERENCES ctdn_p1(id));
