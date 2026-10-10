-- The fold reaches a nested number: `{"a": 1.00}` duplicates `{"a": 1}`.
-- @profile-lock
-- @setup
CREATE TABLE ckn_j (id INT, j JSONB UNIQUE);
INSERT INTO ckn_j (j, id) VALUES ('{"a":1}', 1), ('{"a":"1"}', 2);
-- @test
-- @expect-error: a nested scale-folded jsonb duplicate (23505)
-- @expect-message
INSERT INTO ckn_j (j, id) VALUES ('{"a":1.00}', 3);
