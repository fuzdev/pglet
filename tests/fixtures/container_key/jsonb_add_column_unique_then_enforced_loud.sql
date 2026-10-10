-- `ALTER TABLE … ADD COLUMN … JSONB … UNIQUE` with a default over an existing row
-- back-fills the key, and the index then refuses its duplicate at another scale.
-- @profile-lock
-- @setup
CREATE TABLE ckl_j (id INT);
INSERT INTO ckl_j VALUES (1);
ALTER TABLE ckl_j ADD COLUMN j JSONB DEFAULT '{"k":1}' UNIQUE;
-- @test
-- @expect-error: a duplicate of the back-filled default (23505)
-- @expect-message
INSERT INTO ckl_j (j, id) VALUES ('{"k":1.0}', 2);
