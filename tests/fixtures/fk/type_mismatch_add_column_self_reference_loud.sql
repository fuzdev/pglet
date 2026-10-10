-- A self-referential `ADD COLUMN … REFERENCES` is typed against the table's own key.
-- @setup
CREATE TABLE fts (id INT PRIMARY KEY);

-- @expect-error: 42804 text referencing integer
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE fts ADD COLUMN parent TEXT REFERENCES fts;
