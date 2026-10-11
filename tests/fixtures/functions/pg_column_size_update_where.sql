-- An `UPDATE`'s `WHERE` reads the target's stored row, so a short value is sized under
-- its one-byte header there too.
-- @setup
CREATE TABLE pcu (id int, b bytea);
INSERT INTO pcu VALUES (1, '\xdeadbeef'), (2, '\xdeadbeefdeadbeef');
UPDATE pcu SET id = id + 10 WHERE pg_column_size(b) = 5;
-- @test
SELECT id FROM pcu ORDER BY id;
