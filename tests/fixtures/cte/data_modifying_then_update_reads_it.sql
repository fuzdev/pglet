-- An `UPDATE` body reading a data-modifying `WITH` on an unrelated table.
-- @setup
CREATE TABLE dmu_log (id SERIAL, note TEXT);
CREATE TABLE dmu (k INT, v TEXT);
INSERT INTO dmu VALUES (1, 'a'), (2, 'b');
-- @test
WITH c AS (INSERT INTO dmu_log (note) VALUES ('touch') RETURNING id)
UPDATE dmu SET v = v || c.id FROM c WHERE dmu.k = 1 RETURNING dmu.k, dmu.v;
