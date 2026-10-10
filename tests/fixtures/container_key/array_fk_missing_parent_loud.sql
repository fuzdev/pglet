-- A child array key with no parent — `{1,3}` beside the parent `{1,2}` — is refused.
-- @profile-lock
-- @setup
CREATE TABLE ckmp_a (k INT8[] PRIMARY KEY);
CREATE TABLE ckmc_a (id INT, k INT8[] REFERENCES ckmp_a);
INSERT INTO ckmp_a VALUES ('{1,2}');
-- @test
-- @expect-error: no parent for the child's array key (23503)
-- @expect-message
INSERT INTO ckmc_a (k, id) VALUES ('{1,3}', 1);
