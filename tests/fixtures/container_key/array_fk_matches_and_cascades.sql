-- A foreign key into an array primary key: the child's key matches by array equality,
-- and `ON DELETE CASCADE` follows the parent out.
-- @profile-lock
-- @setup
CREATE TABLE ckfp_a (k INT8[] PRIMARY KEY);
CREATE TABLE ckfc_a (id INT, k INT8[] REFERENCES ckfp_a ON DELETE CASCADE);
INSERT INTO ckfp_a VALUES ('{1,2}'), ('{1,NULL}'), ('{3}');
INSERT INTO ckfc_a (k, id) VALUES ('{1,2}', 1), ('{1,NULL}', 2), ('{3}', 3), (NULL, 4);
DELETE FROM ckfp_a WHERE k = '{1,2}';
DELETE FROM ckfp_a WHERE k = '{1,NULL}';
-- @test
SELECT id, k FROM ckfc_a ORDER BY id;
