-- A child `jsonb` key with no parent is refused.
-- @profile-lock
-- @setup
CREATE TABLE ckmp_j (k JSONB PRIMARY KEY);
CREATE TABLE ckmc_j (id INT, k JSONB REFERENCES ckmp_j);
INSERT INTO ckmp_j VALUES ('{"k":1}');
-- @test
-- @expect-error: no parent for the child's jsonb key (23503)
-- @expect-message
INSERT INTO ckmc_j (k, id) VALUES ('{"k":2}', 1);
