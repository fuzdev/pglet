-- The same `ADD COLUMN` over two rows: the constant default is one key twice, and the
-- statement is refused whole.
-- @profile-lock
-- @setup
CREATE TABLE ckf_j (id INT);
INSERT INTO ckf_j VALUES (1), (2);
-- @test
-- @expect-error: a constant jsonb default over two rows (23505)
-- @expect-message
ALTER TABLE ckf_j ADD COLUMN j JSONB DEFAULT '{"k":1}' UNIQUE;
