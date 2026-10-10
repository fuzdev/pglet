-- A foreign key into a `jsonb` primary key matches as `jsonb`'s `=` does: the child
-- `{"k": 1.0}` references the parent `{"k": 1}`, and a NULL child references nothing.
-- @profile-lock
-- @setup
CREATE TABLE ckfp_j (k JSONB PRIMARY KEY);
CREATE TABLE ckfc_j (id INT, k JSONB REFERENCES ckfp_j);
INSERT INTO ckfp_j VALUES ('{"k":1}'), ('[1,2]');
INSERT INTO ckfc_j (k, id) VALUES ('{"k":1.0}', 1), ('[1.0,2]', 2), (NULL, 3);
-- @test
SELECT id, k::text AS key FROM ckfc_j ORDER BY id;
