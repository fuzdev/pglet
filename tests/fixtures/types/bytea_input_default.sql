-- A column `DEFAULT` string is read as the column's type where the default is stored, at
-- each statement that declares one.
-- @setup
CREATE TABLE bytea_input_d (id INT, a BYTEA DEFAULT 'beef', b BYTEA DEFAULT 'hello', c BYTEA DEFAULT ' ');
ALTER TABLE bytea_input_d ADD COLUMN d BYTEA DEFAULT 'abc';
INSERT INTO bytea_input_d (id) VALUES (1);
ALTER TABLE bytea_input_d ALTER COLUMN a SET DEFAULT '\141\\';
ALTER TABLE bytea_input_d ALTER COLUMN b SET DEFAULT '\x6162 63';
INSERT INTO bytea_input_d (id) VALUES (2);
INSERT INTO bytea_input_d (id, a) VALUES (3, DEFAULT);
-- @test
SELECT id, a, b, c, d FROM bytea_input_d ORDER BY id;
