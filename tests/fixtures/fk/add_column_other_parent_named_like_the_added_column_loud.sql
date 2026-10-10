-- An `ADD COLUMN … REFERENCES` naming **another** table resolves the referenced list in that table's
-- columns only: the column being added belongs to the child, so a referenced name equal to it is a
-- missing column (`42703`), not a covering-key refusal (`42830`).
-- @setup
CREATE TABLE gp (k INT PRIMARY KEY);
CREATE TABLE gc (x INT);

-- @expect-error: 42703 column referenced in foreign key constraint does not exist
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE gc ADD COLUMN a INT REFERENCES gp (a);
