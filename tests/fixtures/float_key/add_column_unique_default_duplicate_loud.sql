-- The same over two rows: the constant default duplicates itself.
-- @profile-lock
-- @expect-error: one default, two rows (23505)
-- @expect-message
-- @setup
CREATE TABLE fka (a INT);
INSERT INTO fka VALUES (1), (2);
-- @test
ALTER TABLE fka ADD COLUMN f REAL UNIQUE DEFAULT 1.5;
