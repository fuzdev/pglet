-- The relation namespace is `pg_class` — tables *and* indexes — so a constraint index
-- named after an existing TABLE is 42P07 too, not just one named after an existing
-- index. A gate that queried only the index map would accept it.
-- @setup
CREATE TABLE ctnt_taken (x INT);

-- @expect-error: relation ctnt_taken already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctnt (id INT NOT NULL, CONSTRAINT ctnt_taken PRIMARY KEY (id));
