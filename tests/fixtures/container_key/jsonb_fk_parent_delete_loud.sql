-- Deleting a `jsonb` parent key a child still references — at another scale — is refused.
-- @profile-lock
-- @setup
CREATE TABLE ckdp_j (k JSONB PRIMARY KEY);
CREATE TABLE ckdc_j (id INT, k JSONB REFERENCES ckdp_j);
INSERT INTO ckdp_j VALUES ('{"k":1}'), ('{"k":2}');
INSERT INTO ckdc_j (k, id) VALUES ('{"k":1.0}', 1);
-- @test
-- @expect-error: the parent key is still referenced (23503)
-- @expect-message
DELETE FROM ckdp_j WHERE k = '{"k":1}';
