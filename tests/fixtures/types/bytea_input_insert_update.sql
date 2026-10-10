-- A string written into a `bytea` column is read by the same rule at each write: a `VALUES`
-- cell, an `INSERT … SELECT` source, an `UPDATE … SET`, an `ON CONFLICT DO UPDATE`.
-- @profile-lock
-- @setup
CREATE TABLE bytea_input_w (id INT PRIMARY KEY, b BYTEA);
INSERT INTO bytea_input_w VALUES
  (1, 'abcd'), (2, '\xabcd'), (3, ' '), (4, 'a\\b'), (5, '\000\377'), (6, 'hello'), (7, '\x6162 63'), (8, '');
INSERT INTO bytea_input_w SELECT 9, 'cafe';
INSERT INTO bytea_input_w SELECT id + 10, 'beef' FROM bytea_input_w WHERE id < 3;
UPDATE bytea_input_w SET b = 'face' WHERE id = 3;
UPDATE bytea_input_w SET b = '\146ace' WHERE id = 8;
INSERT INTO bytea_input_w VALUES (6, 'x') ON CONFLICT (id) DO UPDATE SET b = 'dead';
-- @test
SELECT id, b, length(b) AS n FROM bytea_input_w ORDER BY id;
