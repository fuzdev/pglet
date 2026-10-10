-- UNIQUE and FOREIGN KEY, with the FK written *first*: the constraint indexes are
-- built before the foreign keys are added, so the UNIQUE takes the name and the FK
-- collides with it — 42710 regardless of source order.
-- @setup
CREATE TABLE ctuf_p (id INT PRIMARY KEY);

-- @expect-error: constraint k for relation ctuf already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctuf (a INT, CONSTRAINT k FOREIGN KEY (a) REFERENCES ctuf_p (id),
  CONSTRAINT k UNIQUE (a));
