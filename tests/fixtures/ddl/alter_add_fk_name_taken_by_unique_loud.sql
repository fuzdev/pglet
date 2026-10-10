-- ...and an existing UNIQUE constraint's name blocks an `ADD CONSTRAINT … FOREIGN
-- KEY` (42710).
-- @setup
CREATE TABLE aafu_p (id INT NOT NULL, CONSTRAINT aafu_p_pk PRIMARY KEY (id));
CREATE TABLE aafu (a INT NOT NULL, b INT, CONSTRAINT aafu_uq UNIQUE (a));

-- @expect-error: constraint aafu_uq for relation aafu already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE aafu ADD CONSTRAINT aafu_uq FOREIGN KEY (b) REFERENCES aafu_p (id);
